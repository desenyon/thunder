use std::path::Path;

use anyhow::Result;
use serde_json::Value;

pub fn collect_project_scripts(cwd: &Path) -> Result<Vec<(String, String)>> {
    let mut scripts = Vec::new();
    scripts.extend(parse_package_json(cwd)?);
    scripts.extend(parse_makefile(cwd)?);
    Ok(scripts)
}

fn parse_package_json(cwd: &Path) -> Result<Vec<(String, String)>> {
    let path = cwd.join("package.json");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(path)?;
    let value: Value = serde_json::from_str(&text)?;
    let Some(obj) = value.get("scripts").and_then(|v| v.as_object()) else {
        return Ok(Vec::new());
    };
    Ok(obj
        .iter()
        .map(|(name, cmd)| (format!("npm run {name}"), cmd.as_str().unwrap_or("").to_string()))
        .collect())
}

fn parse_makefile(cwd: &Path) -> Result<Vec<(String, String)>> {
    let path = cwd.join("Makefile");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(path)?;
    let mut scripts = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('.') {
            continue;
        }
        if let Some((target, _deps)) = line.split_once(':') {
            let target = target.trim();
            if !target.is_empty() && !target.starts_with('.') && !target.contains('%') {
                scripts.push((format!("make {target}"), format!("make {target}")));
            }
        }
    }
    scripts.truncate(20);
    Ok(scripts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_make_targets() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("Makefile"), "test:\n\tcargo test\n").unwrap();
        let scripts = parse_makefile(temp.path()).unwrap();
        assert!(scripts.iter().any(|(label, _)| label.contains("make test")));
    }
}
