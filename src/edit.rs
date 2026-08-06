// Vendored from the `edit` and `edit-without-waiting` crates via Autobib.
// Original source licensed under CC0-1.0.

use std::{
    env, fs,
    io::{Error, ErrorKind, Result, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub use tempfile::Builder;
use which::which;

static ENV_VARS: &[&str] = &["VISUAL", "EDITOR"];

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
#[rustfmt::skip]
static WAITING_HARDCODED_NAMES: &[&str] = &[
    "sensible-editor", "nano", "pico", "vim", "nvim", "vi", "emacs",
    "code -w", "atom -w", "subl -w", "gedit -w", "gvim",
];

#[cfg(target_os = "macos")]
#[rustfmt::skip]
static WAITING_HARDCODED_NAMES: &[&str] = &[
    "nano", "pico", "vim", "nvim", "vi", "emacs", "open -Wt",
    "code -w", "atom -w", "subl -w", "gvim", "mate",
];

#[cfg(target_os = "windows")]
#[rustfmt::skip]
static WAITING_HARDCODED_NAMES: &[&str] = &[
    "code.cmd -n -w", "atom.exe -w", "subl.exe -w", "notepad.exe",
];

fn string_to_cmd(value: String) -> (PathBuf, Vec<String>) {
    match shell_words::split(&value) {
        Ok(mut parts) if !parts.is_empty() => (parts.remove(0).into(), parts),
        _ => {
            let mut parts = value.split_ascii_whitespace();
            (
                parts.next().unwrap().into(),
                parts.map(String::from).collect(),
            )
        }
    }
}

fn get_full_editor_cmd(value: String) -> Result<(PathBuf, Vec<String>)> {
    let (path, args) = string_to_cmd(value);
    match which(&path) {
        Ok(result) => Ok((result, args)),
        Err(_) if path.exists() => Ok((path, args)),
        Err(_) => Err(Error::from(ErrorKind::NotFound)),
    }
}

fn get_editor_args() -> Result<(PathBuf, Vec<String>)> {
    ENV_VARS
        .iter()
        .filter_map(env::var_os)
        .filter(|value| !value.is_empty())
        .filter_map(|value| value.into_string().ok())
        .filter_map(|value| get_full_editor_cmd(value).ok())
        .next()
        .or_else(|| {
            WAITING_HARDCODED_NAMES
                .iter()
                .map(|value| (*value).to_owned())
                .filter_map(|value| get_full_editor_cmd(value).ok())
                .next()
        })
        .ok_or_else(|| Error::from(ErrorKind::NotFound))
}

pub fn edit_with_builder<S: AsRef<[u8]>>(text: S, builder: &Builder) -> Result<String> {
    String::from_utf8(edit_bytes_with_builder(text, builder)?)
        .map_err(|_| Error::from(ErrorKind::InvalidData))
}

fn edit_bytes_with_builder<B: AsRef<[u8]>>(buffer: B, builder: &Builder) -> Result<Vec<u8>> {
    let mut file = builder.tempfile()?;
    file.write_all(buffer.as_ref())?;
    let path = file.into_temp_path();
    edit_file(&path)?;
    let edited = fs::read(&path)?;
    path.close()?;
    Ok(edited)
}

fn edit_file<P: AsRef<Path>>(file: P) -> Result<()> {
    let (editor, args) = get_editor_args()?;
    let status = Command::new(&editor)
        .args(&args)
        .arg(file.as_ref())
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .output()?
        .status;

    if status.success() {
        Ok(())
    } else {
        Err(Error::other(format!(
            "editor '{}' exited with error: {status}",
            editor.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_editor_commands() {
        let (path, args) =
            string_to_cmd(r#""/Applications/Foo Bar.app/bin/editor" --wait "set ft=toml""#.into());
        assert_eq!(path, PathBuf::from("/Applications/Foo Bar.app/bin/editor"));
        assert_eq!(args, ["--wait", "set ft=toml"]);
    }
}
