use anyhow::Result;
use rusqlite::Connection;

mod v0;
mod v1;

pub(super) const LATEST_VERSION: i32 = 2;

pub(super) fn migrate(connection: &mut Connection, mut version: i32) -> Result<()> {
    while version < LATEST_VERSION {
        match version {
            0 => v0::migrate(connection)?,
            1 => v1::migrate(connection)?,
            _ => unreachable!("unsupported database migration source version {version}"),
        }
        version += 1;
    }
    Ok(())
}
