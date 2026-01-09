use anyhow::{Context, Result};
use reqwest::blocking::Client;
use reqwest::header::LAST_MODIFIED;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime};
use xmltree::{Element, EmitterConfig, XMLNode};

// ==========================================
// 配置与结构体
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
    username: Option<String>,
    username_cmd: Option<String>,
    password: Option<String>,
    password_cmd: Option<String>,
}

const SEPARATOR: &str = " 📂 ";
const FOLDER_BAR: &str = "Bookmarks Bar";
const FOLDER_MENU: &str = "Bookmarks Menu";
const FOLDER_MOBILE: &str = "Mobile Bookmarks";
const FOLDER_OTHER: &str = "Other Bookmarks";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Bookmark {
    url: String,
    title: String,
    path: Vec<String>,
}

// ==========================================
// 辅助工具
// ==========================================

// 处理 WebDAV URL，自动补全 .xbel
fn get_target_url(raw_url: &str) -> String {
    let trimmed = raw_url.trim();
    if trimmed.to_lowercase().ends_with(".xbel") {
        trimmed.to_string()
    } else {
        let base = trimmed.trim_end_matches('/');
        format!("{}/bookmarks.xbel", base)
    }
}

// ==========================================
// 1. 解析逻辑 (Reader)
// ==========================================

// 解析 Qutebrowser 本地文件
fn parse_local_file(path: &PathBuf) -> Result<HashMap<String, Bookmark>> {
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

        if let Some((url, raw_rest)) = line.split_once(' ') {
            let parts: Vec<&str> = raw_rest.split(SEPARATOR).collect();
            let (path_vec, title) = if parts.len() > 1 {
                let t = parts.last().unwrap().to_string();
                let p = parts[0..parts.len() - 1]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                (p, t)
            } else {
                (vec![], raw_rest.to_string())
            };

            map.insert(
                url.to_string(),
                Bookmark {
                    url: url.to_string(),
                    title,
                    path: path_vec,
                },
            );
        }
    }
    Ok(map)
}

// 防止多重转义导致的幽灵删除
fn html_decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut current = s.to_string();
    let mut limit = 0;
    loop {
        if !current.contains('&') {
            break;
        }
        let next = current
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'");
        if next == current {
            break;
        }
        current = next;
        limit += 1;
        if limit > 10 {
            break;
        }
    }
    current
}

// 这里的 Map 仅用于逻辑对比 (Diff)，不会用于回写
fn parse_remote_xml_dom(xml_content: &str) -> Result<HashMap<String, Bookmark>> {
    let mut map = HashMap::new();
    // 如果文件为空，直接返回空 map
    if xml_content.trim().is_empty() {
        return Ok(map);
    }

    let root = Element::parse(xml_content.as_bytes())?;
    let mut path_stack: Vec<String> = Vec::new();

    // 递归遍历 DOM 树
    fn traverse(elem: &Element, stack: &mut Vec<String>, map: &mut HashMap<String, Bookmark>) {
        if elem.name == "folder" {
            let title = elem
                .get_child("title")
                .and_then(|t| t.get_text())
                .unwrap_or(std::borrow::Cow::Borrowed("Untitled"))
                .to_string();

            stack.push(title);
            for child in &elem.children {
                if let XMLNode::Element(e) = child {
                    traverse(e, stack, map);
                }
            }
            stack.pop();
        } else if elem.name == "bookmark" {
            if let Some(raw_href) = elem.attributes.get("href") {
                // 这里的 raw_href 已经被 xmltree 解码过一次了，
                // 但为了防止双重转义脏数据，必须再次通过 html_decode 清洗
                let href = html_decode(raw_href);

                let raw_title = elem
                    .get_child("title")
                    .and_then(|t| t.get_text())
                    .unwrap_or(std::borrow::Cow::Borrowed(""))
                    .to_string();
                // 标题也清洗一下
                let title = html_decode(&raw_title);

                let mut p = stack.clone();
                if !p.is_empty() && p[0] == FOLDER_BAR {
                    p.remove(0);
                }

                map.insert(
                    href.clone(),
                    Bookmark {
                        url: href, // 使用清洗后的 url
                        title,     // 使用清洗后的 title
                        path: p,
                    },
                );
            }
        } else if elem.name == "xbel" {
            for child in &elem.children {
                if let XMLNode::Element(e) = child {
                    traverse(e, stack, map);
                }
            }
        }
    }

    traverse(&root, &mut path_stack, &mut map);
    Ok(map)
}

// 查重功能 (使用 DOM 解析，绝对安全且准确)
fn check_duplicates(local_path: &PathBuf, xml_content: &str) -> Result<()> {
    println!("🔍 Checking for duplicates...");
    let mut found_issues = false;

    // 1. 检查本地重复
    if local_path.exists() {
        let reader = BufReader::new(File::open(local_path)?);
        let mut local_tracker: HashMap<String, Vec<usize>> = HashMap::new();

        for (idx, line) in reader.lines().enumerate() {
            let line = line?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((url, _)) = line.split_once(' ') {
                local_tracker
                    .entry(url.to_string())
                    .or_default()
                    .push(idx + 1);
            }
        }

        for (url, lines) in local_tracker {
            if lines.len() > 1 {
                found_issues = true;
                println!("⚠️  [Local Duplicate] {}", url);
                println!("    Lines: {:?}", lines);
            }
        }
    }

    // 2. 检查远程重复
    let root = Element::parse(xml_content.as_bytes())?;
    let mut remote_tracker: HashMap<String, Vec<String>> = HashMap::new();
    let mut path_stack: Vec<String> = Vec::new();

    fn traverse_check(
        elem: &Element,
        stack: &mut Vec<String>,
        tracker: &mut HashMap<String, Vec<String>>,
    ) {
        if elem.name == "folder" {
            let title = elem
                .get_child("title")
                .and_then(|t| t.get_text())
                .unwrap_or(std::borrow::Cow::Borrowed("Untitled"))
                .to_string();

            stack.push(title);
            for child in &elem.children {
                if let XMLNode::Element(e) = child {
                    traverse_check(e, stack, tracker);
                }
            }
            stack.pop();
        } else if elem.name == "bookmark" {
            if let Some(raw_href) = elem.attributes.get("href") {
                let href = html_decode(raw_href);
                let location = if stack.is_empty() {
                    "Root".to_string()
                } else {
                    stack.join(" > ")
                };
                tracker.entry(href).or_default().push(location);
            }
        } else if elem.name == "xbel" {
            for child in &elem.children {
                if let XMLNode::Element(e) = child {
                    traverse_check(e, stack, tracker);
                }
            }
        }
    }

    traverse_check(&root, &mut path_stack, &mut remote_tracker);

    for (url, locs) in remote_tracker {
        if locs.len() > 1 {
            found_issues = true;
            println!("⚠️  [Remote Duplicate] {}", url);
            for (i, loc) in locs.iter().enumerate() {
                println!("    {}. {}", i + 1, loc);
            }
        }
    }

    if !found_issues {
        println!("✅ No duplicates found.");
    } else {
        println!("❗ Note: Qutebrowser will use the LAST occurrence locally. Remote WebDAV duplicates are PRESERVED (not merged).");
    }
    Ok(())
}

// ==========================================
// 2. DOM 操作逻辑 (Writer)
// ==========================================

fn find_max_id(element: &Element) -> i32 {
    let mut max_id = 0;
    if let Some(id_str) = element.attributes.get("id") {
        if let Ok(id_val) = id_str.parse::<i32>() {
            max_id = max_id.max(id_val);
        }
    }
    for child in &element.children {
        if let XMLNode::Element(child_elem) = child {
            max_id = max_id.max(find_max_id(child_elem));
        }
    }
    max_id
}

fn ensure_path_recursive<'a>(
    current_node: &'a mut Element,
    path_segments: &[String],
    id_counter: &mut i32,
) -> Result<&'a mut Element> {
    if path_segments.is_empty() {
        return Ok(current_node);
    }
    let folder_name = &path_segments[0];

    let mut exists = false;
    for child in &current_node.children {
        if let XMLNode::Element(elem) = child {
            if elem.name == "folder" {
                if let Some(title_elem) = elem.get_child("title") {
                    if let Some(text) = title_elem.get_text() {
                        if text == *folder_name {
                            exists = true;
                            break;
                        }
                    }
                }
            }
        }
    }

    if !exists {
        let new_id = match folder_name.as_str() {
            FOLDER_MOBILE => 1,
            FOLDER_BAR => 2,
            FOLDER_MENU => 3,
            FOLDER_OTHER => 4,
            _ => {
                *id_counter += 1;
                *id_counter
            }
        };
        if new_id > *id_counter {
            *id_counter = new_id;
        }

        let mut new_folder = Element::new("folder");
        new_folder
            .attributes
            .insert("id".to_string(), new_id.to_string());
        let mut title_node = Element::new("title");
        title_node.children.push(XMLNode::Text(folder_name.clone()));
        new_folder.children.push(XMLNode::Element(title_node));
        current_node.children.push(XMLNode::Element(new_folder));
    }

    for child in &mut current_node.children {
        if let XMLNode::Element(elem) = child {
            if elem.name == "folder" {
                if let Some(title_elem) = elem.get_child("title") {
                    if let Some(text) = title_elem.get_text() {
                        if text == *folder_name {
                            return ensure_path_recursive(elem, &path_segments[1..], id_counter);
                        }
                    }
                }
            }
        }
    }
    Err(anyhow::anyhow!("Logic error: created folder not found"))
}

fn insert_bookmark_to_dom(root: &mut Element, bm: &Bookmark, id_counter: &mut i32) -> Result<()> {
    let mut full_path = bm.path.clone();
    if full_path.is_empty()
        || (full_path[0] != FOLDER_BAR
            && full_path[0] != FOLDER_OTHER
            && full_path[0] != FOLDER_MOBILE
            && full_path[0] != FOLDER_MENU)
    {
        full_path.insert(0, FOLDER_BAR.to_string());
    }

    let target_folder = ensure_path_recursive(root, &full_path, id_counter)?;

    for child in &target_folder.children {
        if let XMLNode::Element(elem) = child {
            if elem.name == "bookmark" {
                if let Some(href) = elem.attributes.get("href") {
                    if href == &bm.url {
                        return Ok(());
                    }
                }
            }
        }
    }

    *id_counter += 1;
    let mut bm_elem = Element::new("bookmark");
    bm_elem
        .attributes
        .insert("href".to_string(), bm.url.clone());
    bm_elem
        .attributes
        .insert("id".to_string(), id_counter.to_string());
    let mut title_elem = Element::new("title");
    title_elem.children.push(XMLNode::Text(bm.title.clone()));
    bm_elem.children.push(XMLNode::Element(title_elem));

    target_folder.children.push(XMLNode::Element(bm_elem));
    Ok(())
}

fn delete_bookmark_from_dom(element: &mut Element, url: &str) -> bool {
    let initial_len = element.children.len();
    element.children.retain(|child| {
        if let XMLNode::Element(elem) = child {
            if elem.name == "bookmark" {
                if let Some(href) = elem.attributes.get("href") {
                    return href != url;
                }
            }
        }
        true
    });

    if element.children.len() < initial_len {
        return true;
    }

    for child in &mut element.children {
        if let XMLNode::Element(elem) = child {
            if elem.name == "folder" {
                if delete_bookmark_from_dom(elem, url) {
                    return true;
                }
            }
        }
    }
    false
}

// ==========================================
// 3. 凭证与网络
// ==========================================

fn resolve_credential(val: &Option<String>, cmd: &Option<String>, name: &str) -> Result<String> {
    if let Some(v) = val {
        return Ok(v.clone());
    }
    if let Some(c) = cmd {
        let output = if cfg!(target_os = "windows") {
            Command::new("cmd").args(["/C", c]).output()?
        } else {
            Command::new("sh").args(["-c", c]).output()?
        };
        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Cmd failed for {}: {}",
                name,
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        return Ok(String::from_utf8(output.stdout)?.trim().to_string());
    }
    Err(anyhow::anyhow!("Missing config for {}", name))
}

fn fetch_remote(
    config: &AppConfig,
    target_url: &str,
) -> Result<(String, SystemTime, String, String)> {
    let user = resolve_credential(
        &config.webdav.username,
        &config.webdav.username_cmd,
        "username",
    )?;
    let pass = resolve_credential(
        &config.webdav.password,
        &config.webdav.password_cmd,
        "password",
    )?;

    println!("☁️  Fetching remote: {}", target_url);
    let client = Client::new();
    let resp = client
        .get(target_url)
        .basic_auth(&user, Some(&pass))
        .send()?;

    if !resp.status().is_success() {
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(anyhow::anyhow!("CRITICAL: Remote file not found (404) at: {}\nPlease create an empty 'bookmarks.xbel' on your WebDAV server first.", target_url));
        }
        return Err(anyhow::anyhow!("Remote fetch failed: {}", resp.status()));
    }

    let last_modified = resp
        .headers()
        .get(LAST_MODIFIED)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| httpdate::parse_http_date(s).ok())
        .unwrap_or(SystemTime::UNIX_EPOCH);

    Ok((resp.text()?, last_modified, user, pass))
}

// ==========================================
// 4. 主同步逻辑
// ==========================================

fn sync_once(
    config: &AppConfig,
    local_path: &PathBuf,
    snapshot_path: &PathBuf,
    target_url: &str,
) -> Result<()> {
    println!("🔄 Syncing...");

    // 1. 读取各方状态
    let local_map = parse_local_file(local_path)?;
    let (remote_xml_str, _, username, password) = fetch_remote(config, target_url)?;

    let remote_map = parse_remote_xml_dom(&remote_xml_str)?;

    let snapshot_map: HashMap<String, Bookmark> = if snapshot_path.exists() {
        let f = File::open(snapshot_path)?;
        serde_json::from_reader(BufReader::new(f)).unwrap_or_default()
    } else {
        HashMap::new()
    };

    println!(
        "📊 Stats: Local={}, Remote={}, Snapshot={}",
        local_map.len(),
        remote_map.len(),
        snapshot_map.len()
    );

    let mut to_add_to_remote: Vec<Bookmark> = Vec::new();
    let mut to_delete_from_remote: Vec<String> = Vec::new();
    let mut final_local_map = local_map.clone();

    let mut all_urls: HashSet<String> = HashSet::new();
    all_urls.extend(local_map.keys().cloned());
    all_urls.extend(remote_map.keys().cloned());
    all_urls.extend(snapshot_map.keys().cloned());

    let mut dom_dirty = false;
    let mut root_element = Element::parse(remote_xml_str.as_bytes())?;
    let mut max_id = find_max_id(&root_element);

    let fmt_path = |p: &[String]| -> String {
        if p.is_empty() {
            FOLDER_BAR.to_string()
        } else {
            p.join("/")
        }
    };

    for url in all_urls {
        let in_local = local_map.get(&url);
        let in_remote = remote_map.get(&url); // 现在是 100% 准确的
        let in_snap = snapshot_map.get(&url);

        match (in_local, in_remote, in_snap) {
            (Some(l), Some(r), Some(s)) => {
                if l.path != r.path {
                    if l.path != s.path {
                        println!(
                            "   🔄 Moving remote: {} ({} -> {})",
                            l.title,
                            fmt_path(&s.path),
                            fmt_path(&l.path)
                        );
                        to_delete_from_remote.push(url.clone());
                        to_add_to_remote.push(l.clone());
                    } else if r.path != s.path {
                        println!(
                            "   🔄 Moving local: {} ({} -> {})",
                            r.title,
                            fmt_path(&s.path),
                            fmt_path(&r.path)
                        );
                        final_local_map.insert(url.clone(), r.clone());
                    } else {
                        println!("   ⚠️ Conflict move, preferring local: {}", l.title);
                        to_delete_from_remote.push(url.clone());
                        to_add_to_remote.push(l.clone());
                    }
                } else if l.title != r.title {
                    if l.title != s.title {
                        println!("   ✏️ Renaming remote: {} -> {}", s.title, l.title);
                        to_delete_from_remote.push(url.clone());
                        to_add_to_remote.push(l.clone());
                    } else if r.title != s.title {
                        println!("   ✏️ Renaming local: {} -> {}", s.title, r.title);
                        final_local_map.insert(url.clone(), r.clone());
                    }
                }
            }

            (Some(l), Some(r), None) => {
                if l.path != r.path {
                    println!(
                        "   🔄 Syncing structure (prefer local): {} ({} -> {})",
                        l.title,
                        fmt_path(&r.path),
                        fmt_path(&l.path)
                    );
                    to_delete_from_remote.push(url.clone());
                    to_add_to_remote.push(l.clone());
                }
            }

            (Some(l), None, None) => {
                println!("   🚀 Pushing new: {}", l.title);
                to_add_to_remote.push(l.clone());
            }
            (None, Some(r), None) => {
                println!("   📥 Pulling new: {}", r.title);
                final_local_map.insert(url.clone(), r.clone());
            }
            (None, Some(_), Some(_)) => {
                println!("   🗑️ Deleting remote: {}", url);
                to_delete_from_remote.push(url.clone());
            }
            (Some(_), None, Some(_)) => {
                println!("   🗑️ Deleting local: {}", url);
                final_local_map.remove(&url);
            }
            _ => {}
        }
    }

    for url in to_delete_from_remote {
        if delete_bookmark_from_dom(&mut root_element, &url) {
            dom_dirty = true;
        }
    }
    for bm in to_add_to_remote {
        insert_bookmark_to_dom(&mut root_element, &bm, &mut max_id)?;
        dom_dirty = true;
    }

    let mut sorted_bookmarks: Vec<&Bookmark> = final_local_map.values().collect();
    sorted_bookmarks.sort_by(|a, b| a.path.cmp(&b.path).then(a.title.cmp(&b.title)));

    let mut local_lines = Vec::new();
    for bm in sorted_bookmarks {
        let full_title = if bm.path.is_empty() {
            bm.title.clone()
        } else {
            format!("{}{}{}", bm.path.join(SEPARATOR), SEPARATOR, bm.title)
        };
        local_lines.push(format!("{} {}", bm.url, full_title));
    }
    fs::write(local_path, local_lines.join("\n"))?;

    if dom_dirty {
        let mut buffer = Vec::new();
        let xml_cfg = EmitterConfig::new()
            .perform_indent(true)
            .indent_string("  ")
            .write_document_declaration(false);
        buffer.extend_from_slice(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        buffer.extend_from_slice(b"<!DOCTYPE xbel PUBLIC \"+//IDN python.org//DTD XML Bookmark Exchange Language 1.0//EN//XML\" \"http://pyxml.sourceforge.net/topics/dtds/xbel.dtd\">\n");
        root_element.write_with_config(&mut buffer, xml_cfg)?;

        println!("☁️  Uploading changes to WebDAV...");
        Client::new()
            .put(target_url)
            .basic_auth(username, Some(password))
            .body(buffer)
            .send()?;
    }

    let f = File::create(snapshot_path)?;
    serde_json::to_writer(f, &final_local_map)?;

    println!("✅ Sync Complete.");
    Ok(())
}

// ==========================================
// Main
// ==========================================

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let is_check_mode = args.contains(&"--check-dupe".to_string());

    if is_check_mode {
        println!("🔍 Starting Duplicate Check Mode...");
    } else {
        println!("🚀 Starting qb-floccus");
    }

    let home = dirs::home_dir().context("No home dir")?;
    let qb_config_dir = if cfg!(target_os = "windows") {
        home.join("AppData").join("Roaming").join("qutebrowser")
    } else {
        home.join(".config").join("qutebrowser")
    };

    let config_path = qb_config_dir.join("qb-floccus.toml");
    if !config_path.exists() {
        return Err(anyhow::anyhow!("Config file not found: {:?}", config_path));
    }
    let config_str = fs::read_to_string(&config_path)?;
    let config: AppConfig = toml::from_str(&config_str)?;

    let local_path = config
        .local_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or(qb_config_dir.join("bookmarks").join("urls"));

    let cache_dir = if cfg!(target_os = "windows") {
        home.join("AppData").join("Local").join("qb-floccus")
    } else {
        home.join(".cache").join("qb-floccus")
    };
    fs::create_dir_all(&cache_dir)?;
    let snapshot_path = cache_dir.join("snapshot.json");

    let target_url = get_target_url(&config.webdav.url);

    if is_check_mode {
        let (remote_xml, _, _, _) = fetch_remote(&config, &target_url)?;
        check_duplicates(&local_path, &remote_xml)?;
        return Ok(());
    }

    if let Some(interval) = config.interval {
        println!("🚀 Daemon Mode ({}s)...", interval);
        loop {
            if let Err(e) = sync_once(&config, &local_path, &snapshot_path, &target_url) {
                eprintln!("❌ Error: {}", e);
                let err_msg = e.to_string();
                if err_msg.contains("CRITICAL") || err_msg.contains("401") {
                    eprintln!("🛑 Fatal error detected. Daemon stopped.");
                    std::process::exit(1);
                }
            }
            thread::sleep(Duration::from_secs(interval));
        }
    } else {
        sync_once(&config, &local_path, &snapshot_path, &target_url)?;
    }

    Ok(())
}
