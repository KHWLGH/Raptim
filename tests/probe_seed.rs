//! Writes a throwaway data dir for `scripts/probe.ps1`: enough rows that scrolling, searching and
//! icon decoding all have real work to do, so a measurement means something.
//!
//! The directory comes from `DIASCIA_PROBE_SEED` and the test is `#[ignore]`d, so it cannot run by
//! accident against a real data dir.
use diascia::{
    model::{Entry, ShowMode, Target},
    storage::Store,
};

#[test]
#[ignore = "手动夹具：DIASCIA_PROBE_SEED=<dir> cargo test --test probe_seed -- --ignored"]
fn seed_probe_data_dir() {
    let Some(path) = std::env::var_os("DIASCIA_PROBE_SEED") else {
        return;
    };
    let directory = std::path::PathBuf::from(path);
    std::fs::create_dir_all(&directory).expect("probe dir");
    let mut store = Store::open(&directory.join("diascia.db")).expect("open probe db");
    // A fresh database already seeds 常用 / 默认分类, so collections are looked up before they are
    // created rather than assumed absent.
    let ensure = |store: &mut Store, parent: Option<i64>, name: &str| -> i64 {
        store
            .collections()
            .unwrap()
            .into_iter()
            .find(|c| c.name == name && c.parent_id == parent)
            .map_or_else(
                || store.save_collection(None, parent, name).unwrap(),
                |c| c.id,
            )
    };
    let mut categories = Vec::new();
    for group in ["常用", "工作"] {
        let parent = ensure(&mut store, None, group);
        for name in ["默认分类", "工具"] {
            categories.push(ensure(&mut store, Some(parent), name));
        }
    }
    // Existing executables, so icon extraction finds a real resource. URLs are the other shape:
    // addresses are excluded from the startup icon backfill, so the probe stays offline.
    let programs = [
        r"C:\Windows\System32\notepad.exe",
        r"C:\Windows\System32\calc.exe",
        r"C:\Windows\explorer.exe",
        r"C:\Windows\System32\charmap.exe",
    ];
    if !store.entries().unwrap().is_empty() {
        println!("probe dir already has entries, leaving it alone");
        return;
    }
    let mut index = 0;
    for round in 0..10 {
        for category in &categories {
            index += 1;
            let target = if index % 5 == 0 {
                Target::Url {
                    url: format!("https://example.com/probe/{round}-{index}"),
                }
            } else {
                Target::Program {
                    path: programs[index % programs.len()].into(),
                    // A category rejects a second entry with the same target key, so the arguments
                    // are what make these rows distinct.
                    arguments: format!("/probe {index}"),
                    working_dir: String::new(),
                    elevated: false,
                    show: ShowMode::Normal,
                }
            };
            store
                .save_entry(&Entry {
                    id: 0,
                    collection_id: *category,
                    name: format!("探针项目 {index}"),
                    aliases: String::new(),
                    notes: "度量夹具".into(),
                    icon: Vec::new(),
                    position: 0,
                    target,
                })
                .unwrap();
        }
    }
    println!(
        "seeded {} entries into {}",
        store.entries().unwrap().len(),
        directory.display()
    );
}
