// This file is Copyright its original authors, visible in version control history.
//
// This file is licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. You may not use this file except in
// accordance with one or both of these licenses.

//! Logging-related objects.

pub(crate) use lightning::util::logger::{Logger as LdkLogger, Record};
pub(crate) use lightning::{log_bytes, log_debug, log_error, log_info, log_trace};

pub use lightning::util::logger::Level as LogLevel;

use chrono::Utc;
use log::{debug, error, info, trace, warn};

use std::fmt::Debug;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

/// A unit of logging output with Metadata to enable filtering Module_path,
/// file, line to inform on log's source.
pub struct LogRecord {
	/// The verbosity level of the message.
	pub level: LogLevel,
	/// The message body.
	pub args: String,
	/// The module path of the message.
	pub module_path: String,
	/// The line containing the message.
	pub line: u32,
}

impl<'a> From<Record<'a>> for LogRecord {
	fn from(record: Record) -> Self {
		Self {
			level: record.level,
			args: record.args.to_string(),
			module_path: record.module_path.to_string(),
			line: record.line,
		}
	}
}

/// LogWriter trait encapsulating the operations required of a
/// logger's writer.
pub trait LogWriter: Send + Sync + Debug {
	/// Log the record.
	fn log(&self, record: LogRecord);
}

/// Defines a writer for [`Logger`].
#[derive(Debug)]
pub(crate) enum Writer {
	/// Writes logs to the file system.
	FileWriter { file_path: String, level: LogLevel },
	/// Forwards logs to the `log` facade.
	LogFacadeWriter { level: LogLevel },
	/// Forwards logs to a custom writer.
	CustomWriter(Arc<dyn LogWriter + Send + Sync>),
}

impl LogWriter for Writer {
	fn log(&self, record: LogRecord) {
		let log = format!(
			"{} {:<5} [{}:{}] {}\n",
			Utc::now().format("%Y-%m-%d %H:%M:%S"),
			record.level.to_string(),
			record.module_path,
			record.line,
			record.args
		);

		match self {
			Writer::FileWriter { file_path, level } => {
				if record.level < *level {
					return;
				}

				fs::OpenOptions::new()
					.create(true)
					.append(true)
					.open(file_path.clone())
					.expect("Failed to open log file")
					.write_all(log.as_bytes())
					.expect("Failed to write to log file")
			},
			Writer::LogFacadeWriter { level } => match level {
				LogLevel::Gossip => trace!("{}", log),
				LogLevel::Trace => trace!("{}", log),
				LogLevel::Debug => debug!("{}", log),
				LogLevel::Info => info!("{}", log),
				LogLevel::Warn => warn!("{}", log),
				LogLevel::Error => error!("{}", log),
			},
			Writer::CustomWriter(custom_logger) => custom_logger.log(record),
		}
	}
}

pub(crate) struct Logger {
	/// Specifies the logger's writer.
	writer: Writer,
}

impl Logger {
	/// Creates a new logger with a filesystem writer. The parameters to this function
	/// are the path to the log file, and the log level.
	pub fn new_fs_writer(file_path: &str, level: LogLevel) -> Result<Self, ()> {
		if let Some(parent_dir) = Path::new(&file_path).parent() {
			fs::create_dir_all(parent_dir)
				.map_err(|e| eprintln!("ERROR: Failed to create log parent directory: {}", e))?;

			// make sure the file exists.
			fs::OpenOptions::new()
				.create(true)
				.append(true)
				.open(&file_path)
				.map_err(|e| eprintln!("ERROR: Failed to open log file: {}", e))?;
		}

		Ok(Self { writer: Writer::FileWriter { file_path: file_path.to_string(), level } })
	}

	pub fn new_log_facade(level: LogLevel) -> Result<Self, ()> {
		Ok(Self { writer: Writer::LogFacadeWriter { level } })
	}

	pub fn new_custom_writer(log_writer: Arc<dyn LogWriter + Send + Sync>) -> Result<Self, ()> {
		Ok(Self { writer: Writer::CustomWriter(log_writer) })
	}
}

impl LdkLogger for Logger {
	fn log(&self, record: Record) {
		match &self.writer {
			Writer::FileWriter { file_path: _, level } => {
				if record.level < *level {
					return;
				}
				self.writer.log(record.into());
			},
			Writer::LogFacadeWriter { level } => {
				if record.level < *level {
					return;
				}
				self.writer.log(record.into());
			},
			Writer::CustomWriter(_arc) => {
				self.writer.log(record.into());
			},
		}
	}
}
