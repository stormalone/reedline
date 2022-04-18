use std::time::Duration;

use chrono::Utc;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct HistoryItemId(pub(crate) i64);
impl HistoryItemId {
    pub(crate) fn new(i: i64) -> HistoryItemId {
        HistoryItemId(i)
    }
}
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct HistorySessionId(pub(crate) i64);
impl HistorySessionId {
    pub(crate) fn new(i: i64) -> HistorySessionId {
        HistorySessionId(i)
    }
}
/// This trait represents additional arbitrary context to be added to a history (optional, see [HistoryItem])
pub trait HistoryItemExtraInfo: Serialize + DeserializeOwned + Default + Send {}
#[derive(Default, Debug, PartialEq, Eq)]
/// something that is serialized as null and deserialized by ignoring everything
pub struct Anything;
impl Serialize for Anything {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        Option::<Anything>::None.serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Anything {
    fn deserialize<D>(d: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        serde::de::IgnoredAny::deserialize(d).map(|_| Anything)
    }
}
impl HistoryItemExtraInfo for Anything {}
/// Represents one run command with some optional additional context
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryItem<ExtraInfo: HistoryItemExtraInfo = Anything> {
    /// primary key, unique across one history
    pub id: Option<HistoryItemId>,
    /// date-time when this command was started
    pub start_timestamp: Option<chrono::DateTime<Utc>>,
    /// the full command line as text
    pub command_line: String,
    /// a unique id for one shell session.
    /// used so the history can be filtered to a single session
    pub session_id: Option<HistorySessionId>,
    /// the hostname the commands were run in
    pub hostname: Option<String>,
    /// the current working directory
    pub cwd: Option<String>,
    /// the duration the command took to complete
    pub duration: Option<Duration>,
    /// the exit status of the command
    pub exit_status: Option<i64>,
    /// arbitrary additional information that might be interesting
    pub more_info: Option<ExtraInfo>,
}

impl HistoryItem {
    /// create a history item purely from the command line with everything else set to None
    pub fn from_command_line(cmd: impl Into<String>) -> HistoryItem {
        HistoryItem {
            id: None,
            start_timestamp: None,
            command_line: cmd.into(),
            session_id: None,
            hostname: None,
            cwd: None,
            duration: None,
            exit_status: None,
            more_info: None,
        }
    }
}