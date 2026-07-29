//! shurufa-host：桌面常驻进程。
//!
//! `run` 子命令启动剪贴板监听并写入历史库；其余子命令面向历史库的
//! 查询与管理，供验收与后续 UI 面板复用。

mod listener;
mod panel;
mod paste;

use clipboard_store::{ClipEntry, ClipKind, ClipboardStore, RetentionPolicy};
use std::path::PathBuf;

fn db_path() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("shurufa")
        .join("clipboard.db")
}

fn open_store() -> ClipboardStore {
    let path = db_path();
    match ClipboardStore::open(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("打开历史库失败 {}: {e}", path.display());
            std::process::exit(1);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    match cmd {
        "run" => {
            let store = open_store();
            println!("剪贴板监听已启动，历史库：{}", db_path().display());
            if let Err(e) = listener::run(store) {
                eprintln!("监听进程异常退出：{e}");
                std::process::exit(1);
            }
        }
        "list" => {
            let n = parse_arg(&args, 1).unwrap_or(20);
            print_entries(&open_store().list(n, 0).unwrap_or_default());
        }
        "search" => {
            let Some(query) = args.get(1) else {
                eprintln!("用法：shurufa-host search <关键词>");
                std::process::exit(2);
            };
            print_entries(&open_store().search(query, 50).unwrap_or_default());
        }
        "pin" | "unpin" => {
            let Some(id) = parse_arg(&args, 1) else {
                eprintln!("用法：shurufa-host {cmd} <id>");
                std::process::exit(2);
            };
            let ok = open_store()
                .set_pinned(id as i64, cmd == "pin")
                .unwrap_or(false);
            println!("{}", if ok { "已更新" } else { "条目不存在" });
        }
        "delete" => {
            let Some(id) = parse_arg(&args, 1) else {
                eprintln!("用法：shurufa-host delete <id>");
                std::process::exit(2);
            };
            let ok = open_store().delete(id as i64).unwrap_or(false);
            println!("{}", if ok { "已删除" } else { "条目不存在" });
        }
        "copy" => {
            let Some(id) = parse_arg(&args, 1) else {
                eprintln!("用法：shurufa-host copy <id>");
                std::process::exit(2);
            };
            let store = open_store();
            match store.get(id as i64) {
                Ok(Some(entry)) => match paste::copy_entry_to_clipboard(&store, &entry) {
                    Ok(true) => println!("已写回剪贴板"),
                    Ok(false) => println!("条目数据缺失，无法写回"),
                    Err(e) => {
                        eprintln!("写回失败：{e}");
                        std::process::exit(1);
                    }
                },
                _ => println!("条目不存在"),
            }
        }
        "clear" => {
            let n = open_store().clear_unpinned().unwrap_or(0);
            println!("已清空 {n} 条未置顶记录");
        }
        "retention" => {
            let n = open_store()
                .apply_retention(&RetentionPolicy::default())
                .unwrap_or(0);
            println!("清理 {n} 条过期记录");
        }
        _ => {
            println!(
                "用法：shurufa-host <子命令>\n\
                 \x20 run             启动剪贴板监听（常驻）\n\
                 \x20 list [N]        最近 N 条历史（默认 20）\n\
                 \x20 search <关键词>  搜索文本与文件名\n\
                 \x20 pin/unpin <id>  置顶/取消置顶\n\
                 \x20 copy <id>       把条目写回剪贴板\n\
                 \x20 delete <id>     删除单条\n\
                 \x20 clear           清空未置顶记录\n\
                 \x20 retention       立即执行留存清理"
            );
        }
    }
}

fn parse_arg(args: &[String], idx: usize) -> Option<u32> {
    args.get(idx)?.parse().ok()
}

fn print_entries(entries: &[ClipEntry]) {
    if entries.is_empty() {
        println!("（无记录）");
        return;
    }
    for e in entries {
        let kind = match e.kind {
            ClipKind::Text => "文本",
            ClipKind::Image => "图片",
            ClipKind::Files => "文件",
        };
        let pin = if e.pinned { "★" } else { " " };
        let preview = match e.kind {
            ClipKind::Image => format!("<{} 字节>", e.data_size),
            _ => single_line_preview(&e.text, 48),
        };
        println!(
            "{pin}{:>5}  [{kind}] {:<10} {:>8}  {preview}",
            e.id,
            e.source_app,
            age(e.updated_at)
        );
    }
}

/// 压成单行并按字符数截断，避免多行内容打乱列表。
fn single_line_preview(text: &str, max_chars: usize) -> String {
    let mut line: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c })
        .take(max_chars)
        .collect();
    if text.chars().count() > max_chars {
        line.push('…');
    }
    line
}

fn age(ts_millis: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let secs = ((now - ts_millis) / 1000).max(0);
    match secs {
        0..=59 => format!("{secs}秒前"),
        60..=3599 => format!("{}分钟前", secs / 60),
        3600..=86399 => format!("{}小时前", secs / 3600),
        _ => format!("{}天前", secs / 86400),
    }
}
