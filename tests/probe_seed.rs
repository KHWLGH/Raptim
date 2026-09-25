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
    let wanted: usize = std::env::var("DIASCIA_PROBE_ENTRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let blobs: u32 = std::env::var("DIASCIA_PROBE_BLOBS_KB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut index = 0;
    while index < wanted {
        for category in &categories {
            if index == wanted {
                break;
            }
            index += 1;
            let target = if index % 5 == 0 {
                Target::Url {
                    url: format!("https://example.com/probe/{index}"),
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
                    icon: blob(index, blobs),
                    position: 0,
                    target,
                })
                .unwrap();
        }
    }
    let entries = store.entries().unwrap();
    println!(
        "seeded {} entries into {} ({} blob bytes total)",
        entries.len(),
        directory.display(),
        entries.iter().map(|e| e.icon.len()).sum::<usize>()
    );
}

/// A reproducible icon of roughly `kib` KiB. Without this every run carries whatever the shell
/// extraction happened to return, so `entry_blob_bytes` differs between runs and no private-byte
/// number from one run can be compared against another. Each row gets its own bytes so the
/// fingerprints differ the way they do in a real library.
fn blob(index: usize, kib: u32) -> Vec<u8> {
    if kib == 0 {
        return Vec::new();
    }
    // Incompressible noise costs about four bytes per pixel, and `encode` caps at 128px.
    let side = (((kib as usize * 1024) / 4).isqrt().min(128)) as u32;
    let mut seed = 0x9e37_79b9_7f4a_7c15u64 ^ (index as u64).wrapping_mul(0x85eb_ca6b);
    let mut next = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as u8
    };
    let pixels: Vec<u8> = (0..(side * side * 4)).map(|_| next()).collect();
    let image = image::RgbaImage::from_raw(side, side, pixels).expect("fixture image");
    diascia::icons::encode(image).expect("encode fixture")
}
