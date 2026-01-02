use anyhow::{Context, Result};
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;
use reqwest::blocking::Client;
use reqwest::header::LAST_MODIFIED;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Cursor};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, SystemTime};

// ==========================================
// 配置与常量
// ==========================================

#[derive(Debug, Deserialize)]
struct AppConfig {
    local_path: Option<String>,
    interval: Option<u64>,
    webdav: WebDavConfig,
}

#[derive(Debug, Deserialize)]
struct WebDavConfig {
    url: String,
    username: String,
    password: String,
}

const FOLDER_BAR: &str = "Bookmarks Bar";
const FOLDER_OTHER: &str = "Other Bookmarks";
const FOLDER_MENU: &str = "Bookmarks Menu";
const FOLDER_MOBILE: &str = "Mobile Bookmarks";
const SEPARATOR: &str = " 📂 ";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Bookmark {
    url: String,
    title: String,
    path: Vec<String>,
    #[serde(default)]
    order_id: u64,
    #[serde(default)]
    xbel_id: Option<u32>,
}

impl PartialEq for Bookmark {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url && self.title == other.title && self.path == other.path
    }
}

// ==========================================
// 主流程
// ==========================================

fn main() -> Result<()> {
    // 1. 基础环境准备
    let home = dirs::home_dir().context("No home dir")?;

    // Qutebrowser 配置目录
    let qb_config_dir = if cfg!(target_os = "windows") {
        home.join("AppData").join("Roaming").join("qutebrowser")
    } else {
        home.join(".config").join("qutebrowser")
    };

    // 2. 加载配置
    let config_file = qb_config_dir.join("qb-floccus.toml");
    if !config_file.exists() {
        return Err(anyhow::anyhow!(
            "Config file not found!\nPlease create: {:?}\n\nExample content:\n[webdav]\nurl = '...'\nusername = '...'\npassword = '...'",
            config_file
        ));
    }

    println!("⚙️  Loading config: {:?}", config_file);
    let config_content = fs::read_to_string(&config_file)?;
    let config: AppConfig = toml::from_str(&config_content)?;

    // 3. 计算关键路径
    // 本地书签路径
    let qb_file_path = if let Some(p) = &config.local_path {
        PathBuf::from(p)
    } else {
        qb_config_dir.join("bookmarks").join("urls")
    };

    // 快照路径 (XDG Cache)
    let cache_dir = if cfg!(target_os = "windows") {
        home.join("AppData").join("Local").join("qb-floccus")
    } else {
        home.join(".cache").join("qb-floccus")
    };
    if !cache_dir.exists() {
        fs::create_dir_all(&cache_dir)?;
    }
    let snapshot_path = cache_dir.join("snapshot.json");

    // 4. URL 标准化预处理
    let raw_url = config.webdav.url.trim();
    let target_url = if raw_url.to_lowercase().ends_with(".xbel") {
        raw_url.to_string()
    } else {
        let base = raw_url.trim_end_matches('/');
        format!("{}/bookmarks.xbel", base)
    };

    // 5. 调度逻辑 (单次运行 或 守护进程)
    match config.interval {
        Some(secs) if secs > 0 => {
            println!(
                "🚀 Starting qb-floccus in DAEMON mode (Interval: {}s)...",
                secs
            );
            loop {
                // 在循环中捕获错误，防止单次网络波动导致进程崩溃退出
                match sync_once(&config, &target_url, &qb_file_path, &snapshot_path) {
                    Ok(_) => println!("✅ Sync cycle finished. Sleeping for {}s...", secs),
                    Err(e) => eprintln!("❌ Sync failed: {:#?}\n   Retrying in {}s...", e, secs),
                }
                thread::sleep(Duration::from_secs(secs));
            }
        }
        _ => {
            println!("🚀 Starting qb-floccus (Run-Once mode)...");
            // 单次模式直接抛出错误
            sync_once(&config, &target_url, &qb_file_path, &snapshot_path)?;
        }
    }

    Ok(())
}

// ==========================================
// 核心同步逻辑 (Sync Once)
// ==========================================

fn sync_once(
    config: &AppConfig,
    target_url: &str,
    qb_file_path: &PathBuf,
    snapshot_path: &PathBuf,
) -> Result<()> {
    // 1. 读取本地
    println!("📂 Reading local: {:?}", qb_file_path);
    let local_map = parse_qutebrowser_file(qb_file_path).unwrap_or_default();
    let local_mtime = fs::metadata(qb_file_path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);

    // 2. 读取快照
    let snapshot_map: HashMap<String, Bookmark> = if snapshot_path.exists() {
        let f = File::open(snapshot_path)?;
        serde_json::from_reader(BufReader::new(f)).unwrap_or_default()
    } else {
        HashMap::new()
    };

    // 3. 读取远程
    println!("☁️  Fetching remote: {}", target_url);
    let client = Client::new();
    let resp = client
        .get(target_url)
        .basic_auth(&config.webdav.username, Some(&config.webdav.password))
        .send()
        .context("WebDAV connect fail")?;

    let status = resp.status();
    let remote_last_modified = resp
        .headers()
        .get(LAST_MODIFIED)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| httpdate::parse_http_date(s).ok())
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let remote_xml = if status.is_success() {
        resp.text()?
    } else {
        println!("   ⚠️ Remote status {}, assuming empty.", status);
        String::new()
    };

    // 解析远程
    let remote_map = parse_xbel_stream(&remote_xml)?;

    println!(
        "📊 Stats: Local={}, Remote={}, Snapshot={}",
        local_map.len(),
        remote_map.len(),
        snapshot_map.len()
    );

    // 熔断保护
    if status.is_success()
        && !remote_xml.is_empty()
        && remote_map.is_empty()
        && !snapshot_map.is_empty()
    {
        return Err(anyhow::anyhow!(
            "CRITICAL: Remote parsed 0 bookmarks! Logic error."
        ));
    }

    // 遍历远程和快照，找到最大的 xbel_id
    let mut max_xbel_id = 0;
    // 辅助闭包：更新最大值
    let mut update_max = |map: &HashMap<String, Bookmark>| {
        for b in map.values() {
            if let Some(id) = b.xbel_id {
                if id > max_xbel_id {
                    max_xbel_id = id;
                }
            }
        }
    };
    update_max(&remote_map);
    update_max(&snapshot_map);
    // 如果是全新的系统，从 100 开始避免冲突系统保留 ID (1-4)
    if max_xbel_id < 100 {
        max_xbel_id = 100;
    }

    // 4. 合并逻辑
    let mut final_map = HashMap::new();
    let mut all_urls: HashSet<String> = HashSet::new();
    all_urls.extend(local_map.keys().cloned());
    all_urls.extend(remote_map.keys().cloned());
    all_urls.extend(snapshot_map.keys().cloned());

    let local_is_newer = local_mtime >= remote_last_modified;
    let first_run = snapshot_map.is_empty();
    let mut max_order_id = remote_map.values().map(|b| b.order_id).max().unwrap_or(0);

    for url in all_urls {
        let in_local = local_map.get(&url);
        let in_remote = remote_map.get(&url);
        let in_snap = snapshot_map.get(&url);

        let mut chosen: Option<Bookmark> = None;

        if first_run {
            if let Some(l) = in_local {
                chosen = Some(l.clone());
            } else if let Some(r) = in_remote {
                chosen = Some(r.clone());
            }
        } else {
            match (in_local, in_remote, in_snap) {
                (Some(l), Some(r), Some(s)) if l == r && r == s => {
                    chosen = Some(l.clone());
                }
                (Some(l), Some(r), _) => {
                    chosen = Some(if l != r && local_is_newer {
                        l.clone()
                    } else {
                        r.clone()
                    });
                }
                (Some(l), None, None) => {
                    chosen = Some(l.clone());
                }
                (None, Some(r), None) => {
                    chosen = Some(r.clone());
                }
                (None, Some(_), Some(_)) => {
                    println!("   🗑️ Del Remote: {}", url);
                }
                (Some(_), None, Some(_)) => {
                    println!("   🗑️ Del Local: {}", url);
                }
                _ => {}
            }
        }

        // ID 分配与继承逻辑
        if let Some(mut bm) = chosen {
            // A. 处理 Order ID (保持原逻辑)
            let existing_order = in_remote
                .map(|b| b.order_id)
                .or(in_snap.map(|b| b.order_id));
            if let Some(old) = existing_order {
                bm.order_id = old;
            } else {
                max_order_id += 1;
                bm.order_id = max_order_id;
            }

            // B. 处理 XBEL ID (持久化核心)
            // 尝试从远程或快照中获取已有的 ID
            let existing_xbel_id = in_remote
                .and_then(|b| b.xbel_id)
                .or(in_snap.and_then(|b| b.xbel_id));

            if let Some(uid) = existing_xbel_id {
                // 如果以前有 ID，必须继承！
                bm.xbel_id = Some(uid);
            } else {
                // 如果是全新的（本地新建的），分配新 ID
                max_xbel_id += 1;
                bm.xbel_id = Some(max_xbel_id);
            }

            final_map.insert(url.to_string(), bm);
        }
    }

    println!("✨ Final count: {}", final_map.len());

    // 5. 写入本地
    let qb_content = generate_sorted_qutebrowser_content(&final_map);
    fs::write(qb_file_path, qb_content).context("Write local fail")?;

    // 6. 写入远程
    let xbel_content = generate_xbel_content(&final_map)?;
    if xbel_content.len() < 100 && final_map.len() > 2 {
        return Err(anyhow::anyhow!("Generated XBEL too small."));
    }
    client
        .put(target_url)
        .basic_auth(&config.webdav.username, Some(&config.webdav.password))
        .body(xbel_content)
        .send()
        .context("Upload XBEL fail")?;

    // 7. 更新快照
    let f = File::create(snapshot_path)?;
    serde_json::to_writer(f, &final_map)?;

    Ok(())
}

// ==========================================
// 新版解析器 (Quick-XML Stream)
// ==========================================

fn parse_xbel_stream(xml: &str) -> Result<HashMap<String, Bookmark>> {
    if xml.trim().is_empty() {
        return Ok(HashMap::new());
    }

    let mut reader = Reader::from_str(xml);
    reader.trim_text(true);
    reader.expand_empty_elements(true);

    let mut buf = Vec::new();
    let mut map = HashMap::new();
    let mut path_stack: Vec<String> = Vec::new();
    let mut global_counter: u64 = 0;

    let mut current_href: Option<String> = None;
    let mut current_id: Option<u32> = None;
    let mut in_title = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                match e.name().as_ref() {
                    b"folder" => {
                        path_stack.push("Untitled".to_string());
                    }
                    b"bookmark" => {
                        // 临时变量存储 id
                        let mut tmp_id: Option<u32> = None;
                        for attr in e.attributes() {
                            if let Ok(a) = attr {
                                match a.key.as_ref() {
                                    b"href" => {
                                        let raw = String::from_utf8_lossy(&a.value).to_string();
                                        let clean = html_decode(&raw);
                                        current_href = Some(clean);
                                    }
                                    b"id" => {
                                        let s = String::from_utf8_lossy(&a.value);
                                        if let Ok(num) = s.parse::<u32>() {
                                            tmp_id = Some(num);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        current_id = tmp_id;
                    }
                    b"title" => {
                        in_title = true;
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(e)) => {
                if in_title {
                    let txt = e.unescape()?.to_string();
                    if let Some(ref href) = current_href {
                        global_counter += 1;
                        let filtered_path: Vec<String> = path_stack
                            .iter()
                            .filter(|p| {
                                *p != FOLDER_BAR && *p != FOLDER_MENU && *p != FOLDER_MOBILE
                            })
                            .cloned()
                            .collect();
                        map.insert(
                            href.clone(),
                            Bookmark {
                                url: href.clone(),
                                title: txt,
                                path: filtered_path,
                                order_id: global_counter,
                                xbel_id: current_id,
                            },
                        );
                    } else {
                        if let Some(last) = path_stack.last_mut() {
                            *last = txt;
                        }
                    }
                }
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                b"folder" => {
                    path_stack.pop();
                }
                b"bookmark" => {
                    if let Some(href) = current_href.take() {
                        if !map.contains_key(&href) {
                            global_counter += 1;
                            let filtered_path: Vec<String> = path_stack
                                .iter()
                                .filter(|p| {
                                    *p != FOLDER_BAR && *p != FOLDER_MENU && *p != FOLDER_MOBILE
                                })
                                .cloned()
                                .collect();
                            map.insert(
                                href.clone(),
                                Bookmark {
                                    url: href.clone(),
                                    title: href,
                                    path: filtered_path,
                                    order_id: global_counter,
                                    xbel_id: current_id,
                                },
                            );
                        }
                    }
                }
                b"title" => {
                    in_title = false;
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(map)
}

fn html_decode(s: &str) -> String {
    // 1. 快速通道：如果没有 '&'，说明完全无需处理，直接返回
    if !s.contains('&') {
        return s.to_string();
    }

    let mut current = s.to_string();

    // 安全计数器：防止极其罕见的恶意构造（可选，但在生产环境建议保留）
    let mut limit = 0;

    loop {
        // 2. 预检查：如果这一轮连 '&' 都没了，肯定干净了
        if !current.contains('&') {
            break;
        }

        // 3. 执行替换
        let next = current
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'");

        // 4. 核心退出条件：如果替换了一轮，内容没变，说明已经是最简形式（Raw URL）
        // 即使它里面还有 '&' (比如参数分隔符)，也应该停止了
        if next == current {
            break;
        }

        current = next;

        // 防止意外死循环保底 (比如某种特殊编码攻击)
        limit += 1;
        if limit > 10 {
            break;
        }
    }

    current
}

fn parse_qutebrowser_file(path: &PathBuf) -> Result<HashMap<String, Bookmark>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let reader = BufReader::new(File::open(path)?);
    let mut map = HashMap::new();

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some((url, raw_title)) = line.split_once(' ') {
            let parts: Vec<&str> = raw_title.split(SEPARATOR).collect();
            let (path, title) = if parts.len() > 1 {
                let t = parts.last().unwrap().to_string();
                let p = parts[0..parts.len() - 1]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                (p, t)
            } else {
                (vec![], raw_title.to_string())
            };
            map.insert(
                url.to_string(),
                Bookmark {
                    url: url.to_string(),
                    title,
                    path,
                    order_id: 0,
                    xbel_id: None,
                },
            );
        }
    }
    Ok(map)
}

// ==========================================
// 生成器 (Generators)
// ==========================================

fn generate_sorted_qutebrowser_content(map: &HashMap<String, Bookmark>) -> String {
    let mut list: Vec<&Bookmark> = map.values().collect();
    list.sort_by(|a, b| a.path.cmp(&b.path).then(a.title.cmp(&b.title)));

    let mut lines = Vec::new();
    for bm in list {
        let full_title = if bm.path.is_empty() {
            bm.title.clone()
        } else {
            format!("{}{}{}", bm.path.join(SEPARATOR), SEPARATOR, bm.title)
        };
        lines.push(format!("{} {}", bm.url, full_title));
    }
    lines.join("\n")
}

struct XbelNode {
    id: i32,
    title: String,
    children: BTreeMap<String, XbelNode>,
    bookmarks: Vec<Bookmark>,
}
impl XbelNode {
    fn new(title: &str, id: i32) -> Self {
        Self {
            id,
            title: title.to_string(),
            children: BTreeMap::new(),
            bookmarks: Vec::new(),
        }
    }
}

fn generate_xbel_content(map: &HashMap<String, Bookmark>) -> Result<String> {
    let mut writer = Writer::new_with_indent(Cursor::new(Vec::new()), b' ', 2);

    writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;
    writer.write_event(Event::DocType(BytesText::from_escaped(
        "xbel PUBLIC \"+//IDN python.org//DTD XML Bookmark Exchange Language 1.0//EN//XML\" \"http://pyxml.sourceforge.net/topics/dtds/xbel.dtd\""
    )))?;

    let root_start = BytesStart::new("xbel").with_attributes(vec![("version", "1.0")]);
    writer.write_event(Event::Start(root_start))?;

    let mut id_counter = 100;
    let mut root_folders: BTreeMap<String, XbelNode> = BTreeMap::new();
    root_folders.insert(FOLDER_BAR.to_string(), XbelNode::new(FOLDER_BAR, 2));
    root_folders.insert(FOLDER_OTHER.to_string(), XbelNode::new(FOLDER_OTHER, 4));

    for bm in map.values() {
        let mut target_node: &mut XbelNode;
        if bm.path.is_empty() {
            target_node = root_folders.get_mut(FOLDER_BAR).unwrap();
        } else {
            let root_name = &bm.path[0];
            if !root_folders.contains_key(root_name) {
                let id = match root_name.as_str() {
                    FOLDER_MENU => 3,
                    FOLDER_MOBILE => 1,
                    _ => {
                        id_counter += 1;
                        id_counter
                    }
                };
                root_folders.insert(root_name.clone(), XbelNode::new(root_name, id));
            }
            target_node = root_folders.get_mut(root_name).unwrap();

            for sub_name in &bm.path[1..] {
                if !target_node.children.contains_key(sub_name) {
                    id_counter += 1;
                    target_node
                        .children
                        .insert(sub_name.clone(), XbelNode::new(sub_name, id_counter));
                }
                target_node = target_node.children.get_mut(sub_name).unwrap();
            }
        }
        target_node.bookmarks.push(bm.clone());
    }

    fn write_node(
        writer: &mut Writer<Cursor<Vec<u8>>>,
        node: &XbelNode,
        id_gen: &mut i32,
    ) -> Result<()> {
        let mut folder_start = BytesStart::new("folder");
        folder_start.push_attribute(("id", node.id.to_string().as_str()));
        writer.write_event(Event::Start(folder_start))?;

        writer.write_event(Event::Start(BytesStart::new("title")))?;
        writer.write_event(Event::Text(BytesText::new(&node.title)))?;
        writer.write_event(Event::End(BytesEnd::new("title")))?;

        for child in node.children.values() {
            write_node(writer, child, id_gen)?;
        }

        let mut sorted = node.bookmarks.clone();
        sorted.sort_by(|a, b| a.order_id.cmp(&b.order_id));

        for bm in sorted {
            // 不再使用 id_gen 自增，而是使用持久化的 id
            let final_id = bm.xbel_id.unwrap_or_else(|| {
                // 兜底：理论上不该走到这，因为合并阶段都分配了。
                // 如果真没有，临时生成一个
                *id_gen += 1;
                *id_gen as u32
            });

            let mut bm_start = BytesStart::new("bookmark");
            bm_start.push_attribute(("href", bm.url.as_str()));
            // 写入持久化的 ID
            bm_start.push_attribute(("id", final_id.to_string().as_str()));

            writer.write_event(Event::Start(bm_start))?;

            writer.write_event(Event::Start(BytesStart::new("title")))?;
            writer.write_event(Event::Text(BytesText::new(&bm.title)))?;
            writer.write_event(Event::End(BytesEnd::new("title")))?;

            writer.write_event(Event::End(BytesEnd::new("bookmark")))?;
        }
        writer.write_event(Event::End(BytesEnd::new("folder")))?;
        Ok(())
    }

    if let Some(node) = root_folders.get(FOLDER_BAR) {
        write_node(&mut writer, node, &mut id_counter)?;
    }
    if let Some(node) = root_folders.get(FOLDER_OTHER) {
        write_node(&mut writer, node, &mut id_counter)?;
    }
    for (name, node) in &root_folders {
        if name != FOLDER_BAR && name != FOLDER_OTHER {
            write_node(&mut writer, node, &mut id_counter)?;
        }
    }

    writer.write_event(Event::End(BytesEnd::new("xbel")))?;
    Ok(String::from_utf8(writer.into_inner().into_inner())?)
}
