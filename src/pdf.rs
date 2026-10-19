//! The HTML booklet printed to PDF by a headless Chrome / Chromium (or
//! Edge), already installed on most machines: no PDF engine to ship.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{RagError, Result};

/// Environment variable naming the browser to use.
pub const BROWSER_ENV: &str = "RAG_MOBIL_BROWSER";

/// Executables tried, in order, in the `PATH`.
const CANDIDATES: &[&str] = &[
    "chromium",
    "chromium-browser",
    "google-chrome",
    "google-chrome-stable",
    "chrome",
    "microsoft-edge",
    "msedge",
];

/// Usual install locations, outside the `PATH`.
const LOCATIONS: &[&str] = &[
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
];

/// The browser of [`BROWSER_ENV`], else the first one found.
pub fn find_browser() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os(BROWSER_ENV) {
        return Ok(PathBuf::from(path));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    CANDIDATES
        .iter()
        .flat_map(|name| {
            dirs.iter()
                .flat_map(move |dir| [dir.join(name), dir.join(format!("{name}.exe"))])
        })
        .chain(LOCATIONS.iter().map(PathBuf::from))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            RagError::Pdf(format!(
                "aucun navigateur Chrome, Chromium ou Edge trouvé ; indiquez-le avec {BROWSER_ENV}"
            ))
        })
}

/// `file://` address of a local file.
fn file_url(path: &Path) -> String {
    let path = path
        .display()
        .to_string()
        .replace('\\', "/")
        .replace('%', "%25")
        .replace(' ', "%20")
        .replace('#', "%23");
    format!("file:///{}", path.trim_start_matches('/'))
}

/// Prints `html` to PDF with `browser`; `extra_args` are passed before the
/// page (e.g. `--no-sandbox` in a container running as root). Pictures are
/// given a few seconds to load.
pub fn print(html: &str, browser: &Path, extra_args: &[String]) -> Result<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!("rag-mobil-pdf-{}", std::process::id()));
    let io = |path: &Path| {
        let path = path.to_owned();
        move |source| RagError::Io { path, source }
    };
    std::fs::create_dir_all(&dir).map_err(io(&dir))?;
    let page = dir.join("livret.html");
    let pdf = dir.join("livret.pdf");
    let result = (|| {
        std::fs::write(&page, html).map_err(io(&page))?;
        let output = Command::new(browser)
            .args([
                "--headless",
                "--disable-gpu",
                "--no-pdf-header-footer",
                "--virtual-time-budget=15000",
            ])
            .arg(format!("--print-to-pdf={}", pdf.display()))
            .arg(format!("--user-data-dir={}", dir.join("profile").display()))
            .args(extra_args)
            .arg(file_url(&page))
            .output()
            .map_err(io(browser))?;
        match std::fs::read(&pdf) {
            Ok(bytes) if bytes.starts_with(b"%PDF") => Ok(bytes),
            _ => Err(RagError::Pdf(format!(
                "{} n'a pas produit de PDF ({}) : {}",
                browser.display(),
                output.status,
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or_default()
            ))),
        }
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls() {
        assert_eq!(
            file_url(Path::new("/tmp/a b.html")),
            "file:///tmp/a%20b.html"
        );
        assert_eq!(
            file_url(Path::new(r"C:\Temp\livret.html")),
            "file:///C:/Temp/livret.html"
        );
    }
}
