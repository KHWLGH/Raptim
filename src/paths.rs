use crate::model::{Error, Result, Target};
use std::path::{Component, Path, PathBuf};

pub fn normal_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{}", rest))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}
pub fn absolute(value: &str, base: &Path) -> PathBuf {
    let clean = normal_path(Path::new(value));
    let base = normal_path(base);
    let path = clean.as_path();
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let mut out = PathBuf::new();
    for part in joined.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            _ => out.push(part.as_os_str()),
        }
    }
    out
}
/// Tooltip 显示目标：网址与应用标识原样显示，文件路径按数据库目录解析为绝对路径。
pub fn display_target(target: &Target, base: &Path) -> String {
    if matches!(target, Target::Url { .. } | Target::App { .. }) {
        target.value().to_string()
    } else {
        absolute(target.value(), base)
            .to_string_lossy()
            .into_owned()
    }
}
pub fn convert(value: &str, base: &Path, relative: bool) -> Result<String> {
    if value.is_empty() {
        return Ok(String::new());
    }
    let target = absolute(value, base);
    if !relative {
        return Ok(target.to_string_lossy().into_owned());
    }
    let base = normal_path(base);
    let a: Vec<_> = base.components().collect();
    let b: Vec<_> = target.components().collect();
    let common = a
        .iter()
        .zip(&b)
        .take_while(|(a, b)| {
            a.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
        })
        .count();
    if common == 0 {
        return Err(Error::Invalid("目标不在同一驱动器，保留绝对路径".into()));
    }
    let mut out = PathBuf::from(".");
    for _ in common..a.len() {
        out.push("..");
    }
    for p in &b[common..] {
        out.push(p.as_os_str());
    }
    if out == Path::new(".") {
        return Ok(".\\".into());
    }
    Ok(out.to_string_lossy().into_owned())
}
pub fn convert_target(target: &Target, base: &Path, relative: bool) -> Result<Target> {
    let mut out = target.clone();
    match &mut out {
        Target::Program {
            path, working_dir, ..
        } => {
            *path = convert(path, base, relative)?;
            *working_dir = convert(working_dir, base, relative)?;
        }
        Target::File { path } | Target::Folder { path } | Target::Shortcut { path, .. } => {
            *path = convert(path, base, relative)?
        }
        _ => return Err(Error::Invalid("网址和应用标识没有文件路径".into())),
    }
    Ok(out)
}
/// The target as the Shell should see it: path kinds resolved against the database directory,
/// Url and App passed through because they have no path to resolve. `convert_target(_, _, false)`
/// can only fail for those two kinds, so every caller that wanted "absolutise if there is a path"
/// used to spell this out with an `unwrap_or_else` that swallowed the error.
pub fn resolve_for_shell(target: &Target, base: &Path) -> Target {
    convert_target(target, base, false).unwrap_or_else(|_| target.clone())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_and_different_drive() {
        let base = Path::new("C:\\Portable\\Raptim");
        let relative = convert("C:\\Portable\\Tools\\app.exe", base, true).unwrap();
        assert_eq!(
            absolute(&relative, base),
            Path::new("C:\\Portable\\Tools\\app.exe")
        );
        assert!(convert("D:\\app.exe", base, true).is_err());
        assert_eq!(absolute(".\\app.exe", base), base.join("app.exe"));
        assert_eq!(
            convert(
                "C:\\Portable\\Raptim\\app.exe",
                Path::new(r"\\?\C:\Portable\Raptim"),
                true
            )
            .unwrap(),
            r".\app.exe"
        );
    }
    #[test]
    fn shell_resolution_absolutises_paths_and_passes_the_rest_through() {
        let base = Path::new("C:\\Portable\\Raptim");
        for passthrough in [
            Target::Url {
                url: "https://example.com/A.png".into(),
            },
            Target::App {
                aumid: "Example.App_1.0!Entry".into(),
            },
        ] {
            assert_eq!(resolve_for_shell(&passthrough, base), passthrough);
        }
        assert_eq!(
            resolve_for_shell(
                &Target::Program {
                    path: ".\\Tools\\app.exe".into(),
                    arguments: "--flag".into(),
                    working_dir: ".\\Tools".into(),
                    elevated: false,
                    show: Default::default(),
                },
                base
            ),
            Target::Program {
                path: "C:\\Portable\\Raptim\\Tools\\app.exe".into(),
                arguments: "--flag".into(),
                working_dir: "C:\\Portable\\Raptim\\Tools".into(),
                elevated: false,
                show: Default::default(),
            }
        );
    }
    #[test]
    fn tooltip_targets_cover_six_kinds() {
        let base = Path::new("C:\\Portable\\Raptim");
        let cases = [
            (
                Target::Program {
                    path: ".\\Tools\\app.exe".into(),
                    arguments: String::new(),
                    working_dir: String::new(),
                    elevated: false,
                    show: Default::default(),
                },
                "C:\\Portable\\Raptim\\Tools\\app.exe",
            ),
            (
                Target::File {
                    path: "..\\Docs\\手册.pdf".into(),
                },
                "C:\\Portable\\Docs\\手册.pdf",
            ),
            (
                Target::Folder {
                    path: "Downloads".into(),
                },
                "C:\\Portable\\Raptim\\Downloads",
            ),
            (
                Target::Url {
                    url: "https://example.com/path?q=1".into(),
                },
                "https://example.com/path?q=1",
            ),
            (
                Target::Shortcut {
                    path: ".\\links\\tool.lnk".into(),
                    show: Default::default(),
                },
                "C:\\Portable\\Raptim\\links\\tool.lnk",
            ),
            (
                Target::App {
                    aumid: "Example.App_1.0!Entry".into(),
                },
                "Example.App_1.0!Entry",
            ),
        ];
        for (target, expected) in cases {
            let shown = display_target(&target, base);
            assert!(!shown.is_empty(), "名称与目标均不能为空");
            assert_eq!(shown, expected);
        }
    }
}
