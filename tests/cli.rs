use anyhow::Result;
use assert_cmd::Command;
use assert_fs::TempDir;
use predicates::prelude::*;

#[test]
fn cancelled_add_exits_with_error() -> Result<()> {
    let directory = TempDir::new()?;
    let database = directory.path().join("coffeedb.db");

    Command::cargo_bin("coffeedb")?
        .args(["--database", database.to_str().unwrap(), "add", "filter"])
        .env("EDITOR", "true")
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("no data was added"));

    directory.close()?;
    Ok(())
}

#[test]
fn invalid_add_aborted_by_user_exits_with_error() -> Result<()> {
    let directory = TempDir::new()?;
    let database = directory.path().join("coffeedb.db");
    let invalid_record = directory.path().join("invalid.toml");
    std::fs::write(&invalid_record, "this is not valid TOML")?;

    Command::cargo_bin("coffeedb")?
        .args(["--database", database.to_str().unwrap(), "add", "omni"])
        .env("EDITOR", format!("cp {}", invalid_record.display()))
        .write_stdin("n\n")
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(
            predicate::str::contains("Contents invalid")
                .and(predicate::str::contains("no data was added")),
        );

    directory.close()?;
    Ok(())
}

#[test]
fn cancelled_edit_exits_with_error() -> Result<()> {
    let directory = TempDir::new()?;
    let database = directory.path().join("coffeedb.db");
    let record = directory.path().join("record.toml");
    std::fs::write(
        &record,
        r#"name = "Test Coffee"
roaster = "Test Roaster"
flavours = []
"#,
    )?;

    Command::cargo_bin("coffeedb")?
        .args(["--database", database.to_str().unwrap(), "add", "filter"])
        .env("EDITOR", format!("cp {}", record.display()))
        .write_stdin("\n")
        .assert()
        .success();

    Command::cargo_bin("coffeedb")?
        .args(["--database", database.to_str().unwrap(), "edit", "F1"])
        .env("EDITOR", "true")
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("no data was changed"));

    directory.close()?;
    Ok(())
}

#[test]
fn lists_registered_flavours() -> Result<()> {
    let directory = TempDir::new()?;
    let database = directory.path().join("coffeedb.db");
    let record = directory.path().join("record.toml");
    std::fs::write(
        &record,
        r#"name = "Test Coffee"
roaster = "Test Roaster"
flavours = ["Lemon", "Apple", "Apple"]
"#,
    )?;

    Command::cargo_bin("coffeedb")?
        .args(["--database", database.to_str().unwrap(), "add", "filter"])
        .env("EDITOR", format!("cp {}", record.display()))
        .write_stdin("\n")
        .assert()
        .success();

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "list",
            "--flavours",
        ])
        .assert()
        .success()
        .stdout("apple\nlemon\n")
        .stderr(predicate::str::is_empty());

    directory.close()?;
    Ok(())
}

#[test]
fn flavours_conflicts_with_explicit_template() -> Result<()> {
    let directory = TempDir::new()?;
    let database = directory.path().join("coffeedb.db");

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "list",
            "--flavours",
            "--template",
            "{name}",
        ])
        .assert()
        .failure()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(
            predicate::str::contains("--flavours")
                .and(predicate::str::contains("--template"))
                .and(predicate::str::contains("Usage:")),
        );

    directory.close()?;
    Ok(())
}

#[test]
fn invalid_roaster_url_is_reprompted() -> Result<()> {
    let directory = TempDir::new()?;
    let database = directory.path().join("coffeedb.db");
    let record = directory.path().join("record.toml");
    std::fs::write(
        &record,
        r#"name = "Test Coffee"
roaster = "Test Roaster"
flavours = []
"#,
    )?;

    Command::cargo_bin("coffeedb")?
        .args(["--database", database.to_str().unwrap(), "add", "filter"])
        .env("EDITOR", format!("cp {}", record.display()))
        .write_stdin("not a url\nhttps://example.com\n")
        .assert()
        .success()
        .stderr(predicate::str::contains("Invalid URL"));

    directory.close()?;
    Ok(())
}
