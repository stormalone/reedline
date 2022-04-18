use chrono::{TimeZone, Utc};
use rusqlite::{named_params, params, Connection, ToSql};

use super::{
    base::{CommandLineSearch, SearchDirection, SearchQuery},
    History, HistoryItem, HistoryItemId, HistorySessionId, Result,
};

use std::{path::PathBuf, time::Duration};

/// A history that stores the values to an SQLite database.
/// In addition to storing the command, the history can store an additional arbitrary HistoryEntryContext,
/// to add information such as a timestamp, running directory, result...
pub struct SqliteBackedHistory {
    db: rusqlite::Connection,
}

fn deserialize_history_item(row: &rusqlite::Row) -> rusqlite::Result<HistoryItem> {
    let x: Option<String> = row.get("more_info")?;
    Ok(HistoryItem {
        id: Some(HistoryItemId::new(row.get("id")?)),
        start_timestamp: row
            .get::<&str, Option<i64>>("start_timestamp")?
            .map(|e| Utc.timestamp_millis(e)),
        command_line: row.get("command_line")?,
        session_id: row
            .get::<&str, Option<i64>>("session_id")?
            .map(HistorySessionId::new),
        hostname: row.get("hostname")?,
        cwd: row.get("cwd")?,
        duration: row
            .get::<&str, Option<i64>>("duration_ms")?
            .map(|e| Duration::from_millis(e as u64)),
        exit_status: row.get("exit_status")?,
        more_info: x
            .map(|x| {
                serde_json::from_str(&x).map_err(|e| {
                    // hack
                    rusqlite::Error::InvalidColumnType(
                        0,
                        format!("could not deserialize more_info: {e}"),
                        rusqlite::types::Type::Text,
                    )
                })
            })
            .transpose()?,
    })
}

impl History for SqliteBackedHistory {
    fn save(&mut self, mut entry: HistoryItem) -> Result<HistoryItem> {
        /*if entry.id.is_some() {
            return Err("ID must be empty".to_string());
        }*/
        let ret: i64 = self
            .db
            .prepare(
                "insert into history
                               (id,  start_timestamp,  command_line,  session_id,  hostname,  cwd,  duration_ms,  exit_status,  more_info)
                        values (:id, :start_timestamp, :command_line, :session_id, :hostname, :cwd, :duration_ms, :exit_status, :more_info)
                    on conflict (history.id) do update set
                        start_timestamp = excluded.start_timestamp,
                        command_line = excluded.command_line,
                        session_id = excluded.session_id,
                        hostname = excluded.hostname,
                        cwd = excluded.cwd,
                        duration_ms = excluded.duration_ms,
                        exit_status = excluded.exit_status,
                        more_info = excluded.more_info
                    returning id",
            )
            .map_err(|e|e.to_string())?
            .query_row(
                named_params! {
                    ":id": entry.id.map(|id| id.0),
                    ":start_timestamp": entry.start_timestamp.map(|e| e.timestamp_millis()),
                    ":command_line": entry.command_line,
                    ":session_id": entry.session_id.map(|e| e.0),
                    ":hostname": entry.hostname,
                    ":cwd": entry.cwd,
                    ":duration_ms": entry.duration.map(|e| e.as_millis() as i64),
                    ":exit_status": entry.exit_status,
                    ":more_info": entry.more_info.as_ref().map(|e| serde_json::to_string(e).unwrap())
                },
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        entry.id = Some(HistoryItemId::new(ret));
        Ok(entry)
    }

    fn load(&self, id: HistoryItemId) -> Result<HistoryItem> {
        let entry = self
            .db
            .prepare("select * from history where id = :id")
            .map_err(|e| e.to_string())?
            .query_row(named_params! { ":id": id.0 }, deserialize_history_item)
            .map_err(|e| e.to_string())?;
        Ok(entry)
    }

    fn count(&self, query: SearchQuery) -> Result<i64> {
        let (query, params) = self.construct_query(&query, "coalesce(count(*), 0)");
        debug_print_query(&query, &params);
        let params_borrow: Vec<(&str, &dyn ToSql)> = params.iter().map(|e| (e.0, &*e.1)).collect();
        let result: i64 = self
            .db
            .prepare(&query)
            .unwrap()
            .query_row(&params_borrow[..], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        Ok(result)
    }

    fn search(&self, query: SearchQuery) -> Result<Vec<HistoryItem>> {
        let (query, params) = self.construct_query(&query, "*");
        debug_print_query(&query, &params);
        let params_borrow: Vec<(&str, &dyn ToSql)> = params.iter().map(|e| (e.0, &*e.1)).collect();
        let results: Vec<HistoryItem> = self
            .db
            .prepare(&query)
            .unwrap()
            .query_map(&params_borrow[..], deserialize_history_item)
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<HistoryItem>>>()
            .map_err(|e| e.to_string())?;
        Ok(results)
        /* if let Some((next_id, next_command)) = next_id {
            self.cursor.id = next_id;
            self.cursor.command = Some(next_command);
        } else {
            if !backward {
                // forward search resets to none, backwards search doesn't
                self.cursor.command = None;
            }
        }*/
    }

    fn update(
        &mut self,
        id: HistoryItemId,
        updater: &dyn Fn(HistoryItem) -> HistoryItem,
    ) -> Result<()> {
        // in theory this should run in a transaction
        let item = self.load(id)?;
        self.save(updater(item))?;
        Ok(())
    }

    fn delete(&mut self, h: HistoryItemId) -> Result<()> {
        let changed = self
            .db
            .execute("delete from history where id = ?", params![h.0])
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("Could not find item".to_string());
        }
        Ok(())
    }

    fn sync(&mut self) -> std::io::Result<()> {
        // no-op (todo?)
        Ok(())
    }

    fn new_session_id(&mut self) -> Result<HistorySessionId> {
        Ok(HistorySessionId::new(
            self.db
                .query_row(
                    "select coalesce(max(session_id), 0) + 1 from history",
                    params![],
                    |r| r.get(0),
                )
                .map_err(map_sqlite_err)?,
        ))
    }

    /*fn iter_chronologic(&self) -> Box<(dyn DoubleEndedIterator<Item = std::string::String> + '_)> {
        // todo: read in chunks or dynamically (?)
        let fwd = self
            .db
            .prepare("select command from history order by id asc")
            .unwrap()
            .query_map(params![], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<String>>>()
            .unwrap();
        return Box::new(fwd.into_iter());
    }

    fn back(&mut self) {
        self.navigate_in_direction(true)
        // self.cursor.id
    }

    fn forward(&mut self) {
        self.navigate_in_direction(false)
    }

    fn string_at_cursor(&self) -> Option<String> {
        self.cursor.command.clone()
    }

    fn set_navigation(&mut self, navigation: HistoryNavigationQuery) {
        self.cursor.query = navigation;
        self.reset_cursor();
    }

    fn get_navigation(&self) -> HistoryNavigationQuery {
        self.cursor.query.clone()
    }

    fn query_entries(&self, search: &str) -> Vec<String> {
        self.iter_chronologic()
            .rev()
            .filter(|entry| entry.contains(search))
            .collect::<Vec<String>>()
    }

    fn max_values(&self) -> usize {
        self.last_run_command_id.unwrap_or(0) as usize
    }

    /// Writes unwritten history contents to disk.
    ///
    /// If file would exceed `capacity` truncates the oldest entries.
    fn sync(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    /// Reset the internal browsing cursor
    fn reset_cursor(&mut self) {
        // if no command run yet, fetch last id from db
        if self.last_run_command_id == None {
            self.last_run_command_id = self
                .db
                .prepare("select coalesce(max(id), 0) from history")
                .unwrap()
                .query_row(params![], |e| e.get(0))
                .optional()
                .unwrap();
        }
        self.cursor.id = self.last_run_command_id.unwrap_or(0) + 1;
        self.cursor.command = None;
    }*/
}
fn map_sqlite_err(err: rusqlite::Error) -> String {
    // todo: better error mapping
    format!("{:?}", err)
}

type BoxedNamedParams<'a> = Vec<(&'static str, Box<dyn ToSql + 'a>)>;

impl SqliteBackedHistory {
    /// Creates a new history with an associated history file.
    ///
    ///
    /// **Side effects:** creates all nested directories to the file
    ///
    pub fn with_file(file: PathBuf) -> Result<Self> {
        if let Some(base_dir) = file.parent() {
            std::fs::create_dir_all(base_dir).map_err(|e| format!("{}", e))?;
        }
        let db = Connection::open(&file).map_err(map_sqlite_err)?;
        Self::from_connection(db)
    }
    /// Creates a new history in memory
    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory().map_err(map_sqlite_err)?)
    }
    /// initialize a new database / migrate an existing one
    fn from_connection(db: Connection) -> Result<Self> {
        // https://phiresky.github.io/blog/2020/sqlite-performance-tuning/
        db.pragma_update(None, "journal_mode", "wal")
            .map_err(map_sqlite_err)?;
        db.pragma_update(None, "synchronous", "normal")
            .map_err(map_sqlite_err)?;
        db.pragma_update(None, "mmap_size", "1000000000")
            .map_err(map_sqlite_err)?;
        db.pragma_update(None, "foreign_keys", "on")
            .map_err(map_sqlite_err)?;
        db.execute(
            "
        create table if not exists history (
            id integer primary key autoincrement,
            command_line text not null,
            start_timestamp integer,
            session_id integer,
            hostname text,
            cwd text,
            duration_ms integer,
            exit_status integer,
            more_info text
        ) strict;
        create index if not exists idx_history_time on history(start_timestamp);
        create index if not exists idx_history_cwd on history(cwd); -- suboptimal for many hosts
        create index if not exists idx_history_exit_status on history(exit_status);
        create index if not exists idx_history_cmd on history(command_line);
        create index if not exists idx_history_cmd on history(session_id);
        -- todo: better indexes
        ",
            params![],
        )
        .map_err(map_sqlite_err)?;
        Ok(SqliteBackedHistory { db })
    }
    fn construct_query<'a>(
        &self,
        query: &'a SearchQuery,
        select_expression: &str,
    ) -> (String, BoxedNamedParams<'a>) {
        // todo: this whole function could be done with less allocs
        let (is_asc, asc) = match query.direction {
            SearchDirection::Forward => (true, "asc"),
            SearchDirection::Backward => (false, "desc"),
        };
        let mut wheres: Vec<&str> = vec![];
        let mut params: BoxedNamedParams = vec![];
        if let Some(start) = query.start_time {
            wheres.push(if is_asc {
                "timestamp_start > :start_time"
            } else {
                "timestamp_start < :start_time"
            });
            params.push((":start_time", Box::new(start.timestamp_millis())))
        }
        if let Some(end) = query.end_time {
            wheres.push(if is_asc {
                ":end_time >= timestamp_start"
            } else {
                ":end_time <= timestamp_start"
            });
            params.push((":end_time", Box::new(end.timestamp_millis())));
        }
        if let Some(start) = query.start_id {
            wheres.push(if is_asc {
                "id > :start_id"
            } else {
                "id < :start_id"
            });
            params.push((":start_id", Box::new(start.0)))
        }
        if let Some(end) = query.end_id {
            wheres.push(if is_asc {
                ":end_id >= id"
            } else {
                ":end_id <= id"
            });
            params.push((":end_id", Box::new(end.0)));
        }
        let limit = match query.limit {
            Some(l) => {
                params.push((":limit", Box::new(l)));
                "limit :limit"
            }
            None => "",
        };
        if let Some(command_line) = &query.filter.command_line {
            // todo: escape %
            let command_line_like = match command_line {
                CommandLineSearch::Exact(e) => e.to_string(),
                CommandLineSearch::Prefix(prefix) => format!("{prefix}%"),
                CommandLineSearch::Substring(cont) => format!("%{cont}%"),
            };
            wheres.push("command_line like :command_line");
            params.push((":command_line", Box::new(command_line_like)));
        }

        if let Some(str) = &query.filter.not_command_line {
            wheres.push("command_line != :not_cmd");
            params.push((":not_cmd", Box::new(str)));
        }
        if let Some(hostname) = &query.filter.hostname {
            wheres.push("hostname = :hostname");
            params.push((":hostname", Box::new(hostname)));
        }
        if let Some(cwd_exact) = &query.filter.cwd_exact {
            wheres.push("cwd = :cwd");
            params.push((":cwd", Box::new(cwd_exact)));
        }
        if let Some(cwd_prefix) = &query.filter.cwd_prefix {
            wheres.push("cwd like :cwd_like");
            let cwd_like = format!("{cwd_prefix}%");
            params.push((":cwd_like", Box::new(cwd_like)));
        }
        if let Some(exit_successful) = query.filter.exit_successful {
            if exit_successful {
                wheres.push("exit_status = 0");
            } else {
                wheres.push("exit_status != 0");
            }
        }
        let mut wheres = wheres.join(" and ");
        if wheres.is_empty() {
            wheres = "true".to_string();
        }
        let query = format!(
            "select {select_expression} from history
        where
        {wheres}
        order by id {asc} {limit}"
        );
        // aprintln!("query={query}");
        (query, params)
    }
}

fn debug_print_query<'a>(_query: &'a str, _params: &'a [(&str, Box<dyn ToSql + 'a>)]) {
    // eprintln!("SQL: {}; -- {:?}", query, params.iter().map(|(k, e)| (k, e.to_sql().unwrap())).collect::<std::collections::HashMap<_, _>>());
}