pub mod library_db;
pub mod gutencore;
pub mod pipeline;
pub mod organizer;
pub mod analytics;
pub mod sync;
pub mod config;

pub use library_db::{LibraryDb, Book, Author, Series};
pub use gutencore::{GutenAdapter, EpubMetadata};
pub use pipeline::Pipeline;
pub use organizer::Organizer;
pub use analytics::{Analytics, LibraryMetrics, BookHealthReport};
pub use sync::SyncSubsystem;
pub use config::RubricaConfig;
