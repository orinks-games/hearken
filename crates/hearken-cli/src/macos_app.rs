//! The Mac app a release ships: `Game.app`, with the program in
//! `Contents/MacOS`, the game's files in `Contents/Resources` and an
//! `Info.plist`, so a player opens it from Finder and no Terminal window
//! appears. The README and licence go beside the app, where a player finds
//! them.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::project::Project;

/// The oldest macOS Rust builds for Apple Silicon run on.
const MINIMUM_MACOS: &str = "11.0";

/// The icon's name inside `Contents/Resources`.
const ICON: &str = "AppIcon";

/// Where each part of a release goes in its staging folder.
pub struct Layout {
    /// The Mac app, when the release is one.
    pub app: Option<PathBuf>,
    /// The program's folder.
    pub programs: PathBuf,
    /// The game's files: sounds, `sounds.pak` and the like.
    pub resources: PathBuf,
    /// Files for the player to read.
    pub docs: PathBuf,
}

impl Layout {
    /// Everything in `stage`, or, for a Mac app, an app inside it.
    pub fn new(stage: &Path, project: &Project, mac_app: bool) -> Layout {
        if mac_app {
            let app = stage.join(format!("{}.app", project.app_name));
            let contents = app.join("Contents");
            Layout {
                programs: contents.join("MacOS"),
                resources: contents.join("Resources"),
                docs: stage.to_path_buf(),
                app: Some(app),
            }
        } else {
            Layout {
                app: None,
                programs: stage.to_path_buf(),
                resources: stage.to_path_buf(),
                docs: stage.to_path_buf(),
            }
        }
    }

    pub fn create(&self) -> Result<(), String> {
        for dir in [&self.programs, &self.resources, &self.docs] {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("Could not make {}: {e}", dir.display()))?;
        }
        Ok(())
    }

    /// Where an included file or folder goes: a document beside the app,
    /// anything else with the game's files.
    pub fn place(&self, item: &str) -> &Path {
        if is_document(item) {
            &self.docs
        } else {
            &self.resources
        }
    }
}

/// A file a player reads, not one the game does: a README, licence,
/// changelog, or any Markdown or text file.
fn is_document(item: &str) -> bool {
    let name = Path::new(item)
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    ["readme", "license", "licence", "copying", "changelog"]
        .iter()
        .any(|p| name.starts_with(p))
        || name.ends_with(".md")
        || name.ends_with(".txt")
}

/// Writes the app's `Info.plist`, and its icon when the game has one.
pub fn finish(project: &Project, app: &Path) -> Result<(), String> {
    let icon = match &project.icon {
        Some(icon) => {
            let resources = app.join("Contents").join("Resources");
            write_icon(&project.dir.join(icon), &resources)?;
            true
        }
        None => {
            println!("No icon: the Mac app shows the default one. Set `icon` to give it one.");
            false
        }
    };
    let plist = app.join("Contents").join("Info.plist");
    std::fs::write(&plist, info_plist(project, icon))
        .map_err(|e| format!("Could not write {}: {e}", plist.display()))?;
    let checked = Command::new("plutil")
        .args(["-lint", "-s"])
        .arg(&plist)
        .status()
        .is_ok_and(|s| s.success());
    if !checked {
        return Err("The Mac app's Info.plist does not parse.".to_string());
    }
    Ok(())
}

/// The app's `Info.plist`.
pub fn info_plist(project: &Project, icon: bool) -> String {
    let version = bundle_version(&project.version);
    let mut keys = vec![
        ("CFBundleDevelopmentRegion", Value::Text("en")),
        ("CFBundleDisplayName", Value::Text(&project.app_name)),
        ("CFBundleExecutable", Value::Text(&project.bin)),
        ("CFBundleIdentifier", Value::Text(&project.bundle_id)),
        ("CFBundleInfoDictionaryVersion", Value::Text("6.0")),
        ("CFBundleName", Value::Text(&project.app_name)),
        ("CFBundlePackageType", Value::Text("APPL")),
        ("CFBundleShortVersionString", Value::Text(&version)),
        ("CFBundleVersion", Value::Text(&version)),
        (
            "LSApplicationCategoryType",
            Value::Text("public.app-category.games"),
        ),
        ("LSMinimumSystemVersion", Value::Text(MINIMUM_MACOS)),
        ("NSHighResolutionCapable", Value::Yes),
    ];
    if icon {
        keys.push(("CFBundleIconFile", Value::Text(ICON)));
    }
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n",
    );
    for (key, value) in keys {
        out.push_str(&format!("\t<key>{key}</key>\n"));
        match value {
            Value::Text(text) => out.push_str(&format!("\t<string>{}</string>\n", escape(text))),
            Value::Yes => out.push_str("\t<true/>\n"),
        }
    }
    out.push_str("</dict>\n</plist>\n");
    out
}

enum Value<'a> {
    Text(&'a str),
    Yes,
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// The version as macOS wants it: up to three numbers, so `1.2.0-dev.abc`
/// is `1.2.0`.
pub fn bundle_version(version: &str) -> String {
    let parts: Vec<&str> = version
        .split(['-', '+'])
        .next()
        .unwrap_or_default()
        .split('.')
        .take(3)
        .take_while(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        .collect();
    if parts.is_empty() {
        "0.0.0".to_string()
    } else {
        parts.join(".")
    }
}

/// Puts the icon at `resources/AppIcon.icns`: an `.icns` as it is, a
/// picture turned into one with macOS's `sips` and `iconutil`.
fn write_icon(icon: &Path, resources: &Path) -> Result<(), String> {
    if !icon.is_file() {
        return Err(format!("The icon {} is not there.", icon.display()));
    }
    let out = resources.join(format!("{ICON}.icns"));
    if icon
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("icns"))
    {
        std::fs::copy(icon, &out).map_err(|e| format!("Could not copy the icon: {e}"))?;
        return Ok(());
    }
    let set = resources.join(format!("{ICON}.iconset"));
    std::fs::create_dir_all(&set).map_err(|e| e.to_string())?;
    for size in [16u32, 32, 128, 256, 512] {
        for (scale, suffix) in [(1, ""), (2, "@2x")] {
            let px = (size * scale).to_string();
            let name = format!("icon_{size}x{size}{suffix}.png");
            let made = Command::new("sips")
                .args(["-z", &px, &px, "-s", "format", "png"])
                .arg(icon)
                .arg("--out")
                .arg(set.join(name))
                .output()
                .is_ok_and(|o| o.status.success());
            if !made {
                return Err(format!(
                    "sips could not resize the icon {}.",
                    icon.display()
                ));
            }
        }
    }
    let made = Command::new("iconutil")
        .args(["-c", "icns"])
        .arg(&set)
        .arg("-o")
        .arg(&out)
        .status()
        .is_ok_and(|s| s.success());
    let _ = std::fs::remove_dir_all(&set);
    if !made {
        return Err("iconutil could not make the app's icon.".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Project {
        Project {
            dir: PathBuf::from("."),
            target_dir: PathBuf::from("target"),
            package: "wickets".into(),
            bin: "wickets".into(),
            version: "1.1.0".into(),
            default_features: Vec::new(),
            portable: true,
            pack: true,
            seal: false,
            seal_id: None,
            include: Vec::new(),
            rules: None,
            app_name: "Wicket's".into(),
            bundle_id: "net.example.wickets".into(),
            icon: None,
        }
    }

    #[test]
    fn a_mac_app_keeps_its_program_and_files_inside_and_docs_beside() {
        let layout = Layout::new(Path::new("dist/wickets"), &project(), true);
        let app = Path::new("dist/wickets/Wicket's.app");
        assert_eq!(layout.app.as_deref(), Some(app));
        assert_eq!(layout.programs, app.join("Contents/MacOS"));
        assert_eq!(layout.resources, app.join("Contents/Resources"));
        assert_eq!(layout.place("README.md"), Path::new("dist/wickets"));
        assert_eq!(layout.place("LICENSE"), Path::new("dist/wickets"));
        assert_eq!(layout.place("sounds"), app.join("Contents/Resources"));
        assert_eq!(layout.place("data/levels"), app.join("Contents/Resources"));
    }

    #[test]
    fn other_releases_keep_everything_together() {
        let layout = Layout::new(Path::new("dist/w"), &project(), false);
        assert_eq!(layout.app, None);
        assert_eq!(layout.programs, Path::new("dist/w"));
        assert_eq!(layout.place("sounds"), Path::new("dist/w"));
        assert_eq!(layout.place("README.md"), Path::new("dist/w"));
    }

    #[test]
    fn the_plist_names_the_program_and_escapes_the_name() {
        let plist = info_plist(&project(), false);
        assert!(plist.contains("<key>CFBundleExecutable</key>\n\t<string>wickets</string>"));
        assert!(plist.contains("<string>Wicket&apos;s</string>"));
        assert!(plist.contains("<string>net.example.wickets</string>"));
        assert!(plist.contains("<key>CFBundleShortVersionString</key>\n\t<string>1.1.0</string>"));
        assert!(!plist.contains("CFBundleIconFile"));
        assert!(info_plist(&project(), true).contains("<string>AppIcon</string>"));
    }

    #[test]
    fn bundle_versions_are_numbers_only() {
        assert_eq!(bundle_version("1.1.0"), "1.1.0");
        assert_eq!(bundle_version("1.2.0-dev.abc1234"), "1.2.0");
        assert_eq!(bundle_version("2.0"), "2.0");
        assert_eq!(bundle_version("1.2.3.4"), "1.2.3");
        assert_eq!(bundle_version("beta"), "0.0.0");
    }
}
