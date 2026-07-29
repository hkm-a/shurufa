//! 剪贴板历史存储测试：覆盖写入、去重、搜索、置顶与留存清理。

use clipboard_store::{ClipKind, ClipboardStore, RetentionPolicy};

fn store() -> ClipboardStore {
    ClipboardStore::open_in_memory().expect("打开内存库失败")
}

#[test]
fn text_roundtrip_and_order() {
    let s = store();
    s.insert_text("第一条", "notepad.exe").unwrap().unwrap();
    s.insert_text("第二条", "code.exe").unwrap().unwrap();
    let list = s.list(10, 0).unwrap();
    assert_eq!(list.len(), 2);
    // 时间倒序：后写入的在前
    assert_eq!(list[0].text, "第二条");
    assert_eq!(list[0].kind, ClipKind::Text);
    assert_eq!(list[0].source_app, "code.exe");
}

#[test]
fn duplicate_bumps_instead_of_inserting() {
    let s = store();
    let id1 = s.insert_text("重复内容", "a.exe").unwrap().unwrap();
    s.insert_text("其他", "a.exe").unwrap().unwrap();
    let id2 = s.insert_text("重复内容", "b.exe").unwrap().unwrap();
    assert_eq!(id1, id2, "相同内容应命中原条目");
    assert_eq!(s.count().unwrap(), 2);
    let top = &s.list(1, 0).unwrap()[0];
    assert_eq!(top.id, id1, "重复复制后应回到最前");
    assert_eq!(top.use_count, 2);
    assert_eq!(top.source_app, "b.exe", "来源应更新为最近一次");
}

#[test]
fn rejects_empty_and_oversized() {
    let s = store();
    assert!(s.insert_text("   ", "a.exe").unwrap().is_none());
    let huge = "字".repeat(clipboard_store::MAX_TEXT_BYTES);
    assert!(s.insert_text(&huge, "a.exe").unwrap().is_none());
    assert!(s.insert_image(&[], "a.exe").unwrap().is_none());
    assert!(s.insert_files(&[], "a.exe").unwrap().is_none());
    assert_eq!(s.count().unwrap(), 0);
}

#[test]
fn image_and_files_roundtrip() {
    let s = store();
    let bmp = vec![0x42u8, 0x4D, 1, 2, 3, 4];
    let img_id = s.insert_image(&bmp, "snip.exe").unwrap().unwrap();
    assert_eq!(s.image_data(img_id).unwrap().unwrap(), bmp);

    let paths = vec!["C:\\a\\报告.docx".to_string(), "C:\\b\\图.png".to_string()];
    s.insert_files(&paths, "explorer.exe").unwrap().unwrap();
    let list = s.list(10, 0).unwrap();
    assert_eq!(list[0].kind, ClipKind::Files);
    assert_eq!(list[0].text.lines().count(), 2);
    assert_eq!(list[1].kind, ClipKind::Image);
    assert!(list[1].data_size > 0);
}

#[test]
fn search_matches_substring_and_escapes_wildcards() {
    let s = store();
    s.insert_text("会议纪要 100% 完成", "a.exe").unwrap();
    s.insert_text("购物清单", "a.exe").unwrap();
    assert_eq!(s.search("纪要", 10).unwrap().len(), 1);
    assert_eq!(s.search("100%", 10).unwrap().len(), 1, "通配符须按字面匹配");
    assert_eq!(s.search("不存在", 10).unwrap().len(), 0);
}

#[test]
fn pin_survives_retention_and_clear() {
    let s = store();
    let pinned_id = s.insert_text("置顶的密钥模板", "a.exe").unwrap().unwrap();
    s.set_pinned(pinned_id, true).unwrap();
    for i in 0..20 {
        s.insert_text(&format!("普通条目{i}"), "a.exe").unwrap();
    }
    // 总量上限 5：未置顶仅保留最新 5 条，置顶不占名额也不被清
    let removed = s
        .apply_retention(&RetentionPolicy {
            max_age_days: 90,
            max_entries: 5,
        })
        .unwrap();
    assert_eq!(removed, 15);
    assert_eq!(s.count().unwrap(), 6);
    assert!(s.get(pinned_id).unwrap().is_some());

    // 清空未置顶后仅剩置顶条目
    s.clear_unpinned().unwrap();
    let rest = s.list(10, 0).unwrap();
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].id, pinned_id);
}

#[test]
fn delete_and_get() {
    let s = store();
    let id = s.insert_text("待删除", "a.exe").unwrap().unwrap();
    assert!(s.get(id).unwrap().is_some());
    assert!(s.delete(id).unwrap());
    assert!(s.get(id).unwrap().is_none());
    assert!(!s.delete(id).unwrap(), "重复删除应返回 false");
}

#[test]
fn persists_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("clips.db");
    {
        let s = ClipboardStore::open(&db).unwrap();
        s.insert_text("跨进程可见", "a.exe").unwrap();
    }
    let s = ClipboardStore::open(&db).unwrap();
    assert_eq!(s.list(10, 0).unwrap()[0].text, "跨进程可见");
}
