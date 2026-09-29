//! Terminal actions using the shared store and session worker.
use super::{Action, App, Screen};
use crate::{
    capture::{Failure, Outcome},
    domain::{
        profile::{CredentialsSource, Profile, ProfileSummary},
        secret::Secret,
    },
    error::{Error, Result},
    progress::Progress,
    store::Store,
    ui::{clipboard, form::ProfileForm, worker::Worker},
};
use std::{path::Path, sync::Arc, time::Instant};

impl App {
    pub(in crate::ui) fn execute(&mut self, action: Action, root: &Path) -> Result<Option<Worker>> {
        if matches!(action, Action::None | Action::Quit) {
            return Ok(None);
        }
        if action == Action::Retry
            && matches!(
                self.screen,
                Screen::Error {
                    profile: None,
                    serial: None,
                    ..
                }
            )
        {
            self.load_profiles(root)?;
            self.show(Screen::Profiles);
            return Ok(None);
        }
        let store = Arc::clone(self.store.as_ref().ok_or(Error::Internal)?);
        match action {
            Action::None | Action::Quit => {}
            Action::Open => self.open_selected(&store)?,
            Action::Create => self.create_profile(&store)?,
            Action::ViewSaved => {
                let serial = self.selected_profile()?.serial.clone();
                let (path, export) = store.load_export(&serial)?;
                self.export_modified = store.export_modified(&serial)?;
                let profile = self.take_profile()?;
                self.show(Screen::Result {
                    profile,
                    outcome: Outcome { path, export },
                    reveal: false,
                    fresh: false,
                });
            }
            Action::ChangeKey | Action::ConfirmReplace => {
                self.replace_label_key(&store, action == Action::ConfirmReplace)?;
            }
            Action::ConfirmDelete => {
                let serial = match &self.screen {
                    Screen::DeleteConfirm { serial, .. } => serial.clone(),
                    _ => return Err(Error::Internal),
                };
                store.delete(&serial)?;
                self.profiles = store.list()?;
                self.export_modified = None;
                self.show(Screen::Profiles);
            }
            Action::Start => {
                let profile = self.selected_profile()?.clone();
                if matches!(self.screen, Screen::ConnectNotice(_)) {
                    return self.launch(store, profile);
                }
                self.show(Screen::ConnectNotice(profile));
            }
            Action::Retry => {
                let profile = self.take_profile()?;
                self.show(Screen::ConnectNotice(profile));
            }
            Action::CopyPassword | Action::CopyUsername => {
                if let Screen::Result { outcome, .. } = &self.screen {
                    let secret = if action == Action::CopyPassword {
                        outcome.export.internet.password.expose()
                    } else {
                        outcome.export.internet.username.expose()
                    };
                    clipboard::copy(secret)?;
                    self.copied_at = Some(Instant::now());
                }
            }
        }
        Ok(None)
    }

    pub(in crate::ui) fn load_profiles(&mut self, root: &Path) -> Result<()> {
        if self.store.is_none() {
            self.store = Some(Arc::new(Store::open(root)?));
        }
        self.profiles = self.store.as_ref().ok_or(Error::Internal)?.list()?;
        Ok(())
    }

    fn open_selected(&mut self, store: &Store) -> Result<()> {
        self.export_modified = None;
        let serial = self
            .profiles
            .get(self.profile_list.selected().unwrap_or_default())
            .ok_or(Error::Internal)?
            .clone();
        let opened = store
            .load(&serial)
            .and_then(|profile| Ok((profile, store.export_modified(&serial)?)));
        match opened {
            Ok((profile, modified)) => {
                self.export_modified = modified;
                self.show(Screen::Device(profile.summary()));
            }
            Err(error) => self.show(Screen::Error {
                profile: None,
                serial: Some(serial),
                error,
                progress: Progress::default(),
            }),
        }
        Ok(())
    }

    fn create_profile(&mut self, store: &Store) -> Result<()> {
        let serial = self.form.serial.parse()?;
        let mac = self.form.mac.parse()?;
        let key = Secret::new(std::mem::take(&mut self.form.key));
        let profile = store.create(&Profile::new(serial, mac, key)?)?;
        self.profiles = store.list()?;
        self.export_modified = store.export_modified(&profile.serial)?;
        self.form = ProfileForm::default();
        self.show(Screen::Device(profile.summary()));
        Ok(())
    }

    fn replace_label_key(&mut self, store: &Store, confirmed: bool) -> Result<()> {
        let mut profile = store.load(&self.selected_profile()?.serial)?;
        if profile.credentials_source == CredentialsSource::Server && !confirmed {
            self.show(Screen::ReplaceSaved(profile.summary()));
            return Ok(());
        }
        let key = Secret::new(std::mem::take(&mut self.form.key));
        profile.use_label_key(key)?;
        store.save(&profile)?;
        self.form = ProfileForm::default();
        self.show(Screen::Device(profile.summary()));
        Ok(())
    }

    fn launch(&mut self, store: Arc<Store>, profile: ProfileSummary) -> Result<Option<Worker>> {
        let worker = Worker::start(store, profile.serial.clone(), Arc::clone(&self.device));
        self.show(Screen::Running {
            profile,
            progress: Progress::default(),
            cancelling: false,
            started: Instant::now(),
        });
        Ok(Some(worker))
    }

    pub(in crate::ui) fn complete(&mut self, result: std::result::Result<Outcome, Failure>) {
        let Screen::Running { mut profile, .. } =
            std::mem::replace(&mut self.screen, Screen::Profiles)
        else {
            self.fail(Error::Internal);
            return;
        };
        if let Some(store) = &self.store {
            if let Ok(updated) = store.load(&profile.serial) {
                profile = updated.summary();
            }
            self.export_modified = store.export_modified(&profile.serial).ok().flatten();
        }
        self.show(match result {
            Ok(outcome) => Screen::Result {
                profile,
                outcome,
                reveal: false,
                fresh: true,
            },
            Err(failure) => Screen::Error {
                profile: Some(profile),
                serial: None,
                error: failure.error,
                progress: failure.progress,
            },
        });
    }

    pub(in crate::ui) fn cancel(&mut self, worker: Option<&Worker>) -> bool {
        if let Some(worker) = worker {
            worker.cancel.cancel();
            if let Screen::Running { cancelling, .. } = &mut self.screen {
                *cancelling = true;
            }
            true
        } else {
            false
        }
    }
}
