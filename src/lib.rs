pub mod library_db;
pub mod pipeline;
pub mod organizer;
pub mod analytics;
pub mod sync;
pub mod config;

pub use library_db::{LibraryDb, Book, Author, Series, SeriesStats, TagStats};
pub use pipeline::Pipeline;
pub use organizer::Organizer;
pub use analytics::{Analytics, LibraryMetrics, BookHealthReport};
pub use sync::SyncSubsystem;
pub use config::{RubricaConfig, default_db_path, default_db_url};
