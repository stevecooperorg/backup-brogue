use notify::{watcher, DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("no home dir")]
    NoHomeDir,
    #[error("missing save dir: {0}")]
    MissingDir(PathBuf),
    #[error("notify error")]
    NotifyError(#[from] notify::Error),
    #[error("IO error")]
    IoError(#[from] std::io::Error),
    #[error("unknown error")]
    Unknown,
}

type Result<T> = std::result::Result<T, AppError>;

// Basic logic:
// ====
// There is a save dir. New files appear (e.g. 'Saved #272472511 at depth 1 (easy).broguesave')
// Each save file should be moved out to a backup folder
// When it disappears from the save dir but exists in the backup dir, copy it over
// When it is deleted from the backup dir, consider deleting itf
//

#[derive(PartialEq, Clone, Debug)]
struct Event {
    path: PathBuf,
    event_type: EventType,
}

impl Event {
    fn new(path: PathBuf, event_type: EventType) -> Self {
        Self { path, event_type }
    }
}

#[derive(PartialEq, Clone, Debug)]
enum EventType {
    SaveCreated,
    SaveDeleted,
    BackupCreated,
    BackupDeleted,
}

fn main() -> Result<()> {
    println!("backup-brogue - watches for suspended games then backs them up for later loading, even after death");

    let user_home = dirs::home_dir().ok_or_else(|| AppError::NoHomeDir)?;
    let save_dir = user_home.join("Library/Application Support/Brogue/Brogue CE");
    let backup_dir = user_home.join(".brogue");

    if !backup_dir.exists() {
        std::fs::create_dir_all(&backup_dir)?;
    }

    let mut work_queue = VecDeque::new();

    // treat every existing save as a new save, on startup, to make sure everything is backed up and happy
    for entry in std::fs::read_dir(&save_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            work_queue.push_back(Event {
                path,
                event_type: EventType::SaveCreated,
            });
        }
    }

    for entry in std::fs::read_dir(&backup_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            work_queue.push_back(Event {
                path,
                event_type: EventType::BackupCreated,
            });
        }
    }

    let (save_watcher, save_rx) = watch(save_dir.clone())?;
    let (backup_watcher, backup_rx) = watch(backup_dir.clone())?;

    loop {
        while let Some(event) = work_queue.pop_front() {
            if event.path.extension().unwrap_or_default() != OsString::from("broguesave") {
                continue;
            }

            match event.event_type {
                EventType::SaveCreated => {
                    if !event.path.exists() {
                        println!(
                            "[BACKUP FAIL] save path does not exist: {}",
                            event.path.to_string_lossy()
                        );
                        continue;
                    }

                    let backup_destination =
                        backup_dir.join(event.path.file_name().unwrap_or_default());
                    if !backup_destination.exists() {
                        println!(
                            "[BACKUP] {} => {}",
                            event.path.file_name().unwrap_or_default().to_string_lossy(),
                            backup_destination.to_string_lossy()
                        );
                        let out = std::fs::copy(&event.path, backup_destination)?;
                    }
                }
                EventType::SaveDeleted => {
                    let origin = backup_dir.join(event.path.file_name().unwrap_or_default());
                    if !origin.exists() {
                        println!(
                            "[RESTORE FAIL] cannot restore: matching backup missing from {}",
                            origin.to_string_lossy()
                        );
                        continue;
                    }

                    println!(
                        "[RESTORE] {} => {}",
                        origin.file_name().unwrap_or_default().to_string_lossy(),
                        event.path.to_string_lossy()
                    );
                    let out = std::fs::copy(origin, &event.path)?;
                }
                _ => {}
            }
        }

        while let Ok(event) = save_rx.try_recv() {
            if let DebouncedEvent::Create(path) = event {
                work_queue.push_back(Event::new(path, EventType::SaveCreated));
            } else if let DebouncedEvent::Remove(path) = event {
                work_queue.push_back(Event::new(path, EventType::SaveDeleted));
            }
        }

        while let Ok(event) = backup_rx.try_recv() {
            if let DebouncedEvent::Create(path) = event {
                work_queue.push_back(Event::new(path, EventType::BackupCreated));
            } else if let DebouncedEvent::Remove(path) = event {
                work_queue.push_back(Event::new(path, EventType::BackupDeleted));
            }
        }

        std::thread::sleep(Duration::from_millis(50));
    }
}

fn watch<P: Into<PathBuf>>(path: P) -> Result<(RecommendedWatcher, Receiver<DebouncedEvent>)> {
    let path: PathBuf = path.into();
    if !path.exists() {
        return Result::Err(AppError::MissingDir(path.into()));
    }

    println!("Watching dir: {}", path.to_str().unwrap_or_default());
    let (tx, rx) = channel();
    let mut watcher = watcher(tx, Duration::from_secs(1)).unwrap();
    watcher.watch(path, RecursiveMode::Recursive).unwrap();
    Ok((watcher, rx))
}

fn handle_new_save(path: &Path, backup_dir: &Path) -> Result<()> {
    let path: PathBuf = path.into();

    println!("Handling save: {:?}", path);

    if !path.exists() {
        return Ok(());
    }

    if path.extension().unwrap_or_default() != OsString::from("broguesave") {
        println!("{:?} is not a .broguesave file", path);
        return Ok(());
    }

    // a new save: copy it into the backup dir
    println!("new save created at {}", path.to_str().unwrap_or_default());

    let dest = backup_dir.join(path.file_name().unwrap_or_default());
    println!("should copy to {}", dest.to_str().unwrap_or_default());

    if !dest.exists() {
        let out = std::fs::copy(path, dest)?;
    }

    Ok(())
}

fn restore_missing_save(path: &Path, backup_dir: &Path) -> Result<()> {
    let path: PathBuf = path.into();

    println!("Handling save: {:?}", path);

    if !path.exists() {
        return Ok(());
    }

    if path.extension().unwrap_or_default() != OsString::from("broguesave") {
        println!("{:?} is not a .broguesave file", path);
        return Ok(());
    }

    // a new save: copy it into the backup dir
    println!("new save created at {}", path.to_str().unwrap_or_default());

    let dest = backup_dir.join(path.file_name().unwrap_or_default());
    println!("should copy to {}", dest.to_str().unwrap_or_default());

    if !dest.exists() {
        let out = std::fs::copy(path, dest)?;
    }

    Ok(())
}

// fn handle_new_backup(path: &Path, save_dir: &Path) -> Result<()> {
//     let path: PathBuf = path.into();
//
//     println!("Handling backup: {:?}", path);
//
//     if !path.exists() {
//         return Ok(());
//     }
//
//     if !path.ends_with(".broguesave") {
//         return Ok(());
//     }
//
//     // a backup: copy it into the save dir
//     println!(
//         "new backup created at {}",
//         path.to_str().unwrap_or_default()
//     );
//
//     let dest = save_dir.join(path.file_name().unwrap_or_default());
//     println!("should copy to {}", dest.to_str().unwrap_or_default());
//
//     if !dest.exists() {
//         let out = std::fs::copy(path, dest)?;
//     }
//
//     Ok(())
// }
