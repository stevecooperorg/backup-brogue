use crate::Result;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(PartialEq)]
pub enum DeleteState {
    NotDeleting,
    AwaitingIndex,
    Delete(usize),
}

pub struct App {
    save_dir: PathBuf,
    backup_dir: PathBuf,
    pub delete_state: DeleteState,
    pub state: State,
}

impl App {
    pub fn update_state(&mut self) -> Result<()> {
        let state = get_state(&self.save_dir, &self.backup_dir)?;
        self.state = state;
        Ok(())
    }

    pub fn new(save_dir: PathBuf, backup_dir: PathBuf) -> App {
        App {
            save_dir,
            backup_dir,
            delete_state: DeleteState::NotDeleting,
            state: State::default(),
        }
    }

    fn cp(from: &Path, to: &Path) -> Result<()> {
        if !from.exists() {
            return Ok(());
        }

        if !to.exists() {
            std::fs::copy(&from, &to)?;
        }

        Ok(())
    }

    fn rm(path: &Path) -> Result<()> {
        if path.exists() {
            std::fs::remove_file(path)?;
        }

        Ok(())
    }

    fn reconcile(&mut self) -> Result<()> {
        if let DeleteState::Delete(idx) = &self.delete_state {
            // delete from both;
            if let Some(save) = &self.state.saves.get(*idx) {
                match save {
                    Save::OriginalFileOnly(x) => {
                        Self::rm(x)?;
                    }
                    Save::BackupFileOnly(x) => {
                        Self::rm(x)?;
                    }
                    Save::Both(x, y) => {
                        Self::rm(y)?;
                        Self::rm(x)?;
                    }
                }
                self.delete_state = DeleteState::NotDeleting;
                return Ok(());
            }
        }

        for save in &self.state.saves {
            match save {
                Save::OriginalFileOnly(save) => {
                    let file_name = save.file_name().unwrap_or_default();
                    let backup_destination = self.backup_dir.join(&file_name);
                    Self::cp(save, &backup_destination)?;
                }
                Save::BackupFileOnly(backup) => {
                    let file_name = backup.file_name().unwrap_or_default();
                    let save_destination = self.save_dir.join(&file_name);
                    Self::cp(backup, &save_destination)?;
                }
                Save::Both(_, _) => {}
            }
        }
        Ok(())
    }

    pub fn on_tick(&mut self) {
        self.reconcile().unwrap();
    }
}
#[derive(Default)]
pub struct State {
    pub saves: Vec<Save>,
}

#[derive(Debug, PartialOrd, Ord, PartialEq, Eq)]
pub enum Save {
    OriginalFileOnly(PathBuf),
    BackupFileOnly(PathBuf),
    Both(PathBuf, PathBuf),
}

impl Save {
    fn key(&self) -> String {
        match self {
            Save::OriginalFileOnly(x) => key(x),
            Save::BackupFileOnly(x) => key(x),
            Save::Both(x, _) => key(x),
        }
    }

    fn sort_by(&self) -> SystemTime {
        match self {
            Save::OriginalFileOnly(x) => sort_by(x),
            Save::BackupFileOnly(x) => sort_by(x),
            Save::Both(x, y) => sort_by(x).max(sort_by(y)),
        }
    }
}

fn key(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

fn sort_by(path: &Path) -> SystemTime {
    path.metadata().unwrap().modified().unwrap()
}

impl Display for Save {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let prefix = match self {
            Save::OriginalFileOnly(_) => "S<-xB",
            Save::BackupFileOnly(_) => "Sx->B",
            Save::Both(_, _) => "S<->B",
        };

        let code = match self {
            Save::OriginalFileOnly(_) => "SAVE",
            Save::BackupFileOnly(_) => "BACK",
            Save::Both(_, _) => "SYNC",
        };
        write!(f, "{} {} {}", code, prefix, self.key())
    }
}

pub fn get_state(save_dir: &Path, backup_dir: &Path) -> Result<State> {
    let save_files = files(save_dir)?;
    let backup_files = files(backup_dir)?;
    let mut map: HashMap<String, Save> = HashMap::new();

    for save_file in save_files {
        map.entry(key(&save_file))
            .or_insert_with(|| Save::OriginalFileOnly(save_file.clone()));
    }

    for backup_file in backup_files {
        map.entry(key(&backup_file))
            .and_modify(|s| {
                if let Save::OriginalFileOnly(p) = s {
                    *s = Save::Both(p.clone(), backup_file.clone());
                }
            })
            .or_insert(Save::BackupFileOnly(backup_file));
    }

    let mut saves: Vec<Save> = map.into_values().collect();
    saves.sort_by_key(|x| x.sort_by());

    // pop in a couple of test values
    // saves.push(Save::SaveOnly(PathBuf::from("save-only.broguesave")));
    // saves.push(Save::BackupOnly(PathBuf::from("backup-only.broguesave")));

    Ok(State { saves })
}

fn files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut res = vec![];
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if is_brogue_save(&path) {
            res.push(path);
        }
    }
    Ok(res)
}

fn is_brogue_save(path: &Path) -> bool {
    !path.is_dir()
        && path.extension().unwrap_or_default() == OsStr::new("broguesave")
        && path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
            .starts_with("Saved")
}
