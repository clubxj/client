use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ClientState {
    Unloaded,
    Running,
    Failed,
    Idle,
}

impl Page {
    pub(super) fn client(self) -> Option<u32> {
        match self {
            Self::Client(pid)
            | Self::Store(pid)
            | Self::Queue(pid)
            | Self::Development(pid)
            | Self::History(pid)
            | Self::Editor(Some(pid)) => Some(pid),
            _ => None,
        }
    }
}

impl LoaderApp {
    pub(super) fn go(&mut self, page: Page, run: Option<u64>) {
        self.page = page;
        if run.is_some() {
            self.active_run = run;
        }
        self.sidebar_open = false;
    }

    pub(super) fn page_client(&self) -> Option<u32> {
        match self.page {
            Page::Run(_) => self.run().and_then(Run::pid),
            page => page.client(),
        }
    }

    pub(super) fn detected(&self, pid: u32) -> Option<&Target> {
        self.targets.iter().find(|t| t.client.pid == pid)
    }

    pub(super) fn client_loaded(&self, pid: u32) -> bool {
        self.target_of(pid)
            .is_some_and(|t| t.client.loaded.is_some() && t.client.is_64bit)
    }

    pub(super) fn runs_on(&self, pid: u32) -> impl Iterator<Item = &Run> {
        self.runs.iter().filter(move |r| r.pid() == Some(pid))
    }

    pub(super) fn live_run_on(&self, pid: u32) -> Option<&Run> {
        self.runs_on(pid).filter(|r| r.is_live()).last()
    }

    pub(super) fn ended_runs_on(&self, pid: u32) -> impl Iterator<Item = &Run> {
        self.runs_on(pid).filter(|r| !r.is_live())
    }

    pub(super) fn queue_on(&self, pid: u32) -> Option<&crate::queue::Runner> {
        self.queue_runners.iter().find(|r| r.pid == pid)
    }

    pub(super) fn client_busy(&self, pid: u32) -> bool {
        self.live_run_on(pid).is_some() || self.queue_on(pid).is_some()
    }

    pub(super) fn client_state(&self, pid: u32) -> ClientState {
        if !self.client_loaded(pid) {
            ClientState::Unloaded
        } else if self.queue_on(pid).is_some() || self.runs_on(pid).any(Run::is_running) {
            ClientState::Running
        } else if self
            .ended_runs_on(pid)
            .max_by_key(|r| r.ended)
            .is_some_and(Run::failed)
        {
            ClientState::Failed
        } else {
            ClientState::Idle
        }
    }

    pub(super) fn listed_clients(&self) -> Vec<u32> {
        let mut pids: Vec<u32> = self.targets.iter().map(|t| t.client.pid).collect();
        for pid in self.runs.iter().filter_map(Run::pid) {
            if !pids.contains(&pid) {
                pids.push(pid);
            }
        }
        pids
    }

    fn page_shown(&self, page: Page) -> bool {
        let idle = |pid: u32| self.client_loaded(pid) && !self.client_busy(pid);
        match page {
            Page::Client(pid) => self.listed_clients().contains(&pid),
            Page::Store(pid) | Page::Editor(Some(pid)) => idle(pid),
            Page::Queue(pid) => idle(pid) || self.queue_on(pid).is_some(),
            Page::Development(pid) => self.client_loaded(pid),
            Page::History(pid) => self.ended_runs_on(pid).next().is_some(),
            Page::Run(sub) => self
                .run()
                .is_some_and(|run| sub != RunPage::Feedback || run.store.is_some()),
            Page::Sessions
            | Page::Editor(None)
            | Page::Terminal
            | Page::Accounts
            | Page::Feedback
            | Page::Settings => true,
        }
    }

    pub(super) fn repair_page(&mut self) {
        if self.page_shown(self.page) {
            return;
        }
        let pid = self.page_client().or(self.focus_client);
        self.page = match pid {
            Some(pid) if self.page_shown(Page::Client(pid)) => Page::Client(pid),
            _ => Page::Sessions,
        };
    }

    pub(super) fn context_run(&self) -> Option<u64> {
        if let Page::Run(_) = self.page {
            return self.active_run;
        }
        let pid = self.page.client()?;
        self.live_run_on(pid)
            .or_else(|| self.runs_on(pid).last())
            .map(|r| r.id)
    }

    pub(super) fn icon_target(&self) -> Option<&Target> {
        loaded_in(&self.targets, self.focus_client)
            .or_else(|| self.targets.iter().find(|t| t.client.loaded.is_some()))
    }

    pub(super) fn dev_of(&mut self, pid: u32) -> Option<&mut Inspectors> {
        if self.dev.pid == Some(pid) {
            Some(&mut self.dev)
        } else {
            self.parked_dev.get_mut(&pid)
        }
    }

    pub(super) fn show_dev(&mut self, pid: u32) {
        if self.dev.pid == Some(pid) {
            return;
        }
        let next = self
            .parked_dev
            .remove(&pid)
            .unwrap_or_else(|| Inspectors::new(Some(pid)));
        let mut parked = std::mem::replace(&mut self.dev, next);
        if let Some(tiles) = parked.tiles.take() {
            tiles.finish();
        }
        if let Some(old) = parked.pid {
            self.parked_dev.insert(old, parked);
        }
    }

    pub(super) fn show_editor(&mut self, owner: Option<u32>) {
        if self.editor.owner == owner {
            return;
        }
        let next = self.editors.remove(&owner).unwrap_or_else(|| Editor {
            owner,
            file: self.global_script().clone(),
            ..Default::default()
        });
        let parked = std::mem::replace(&mut self.editor, next);
        self.editors.insert(parked.owner, parked);
    }

    pub(super) fn global_script(&self) -> &ScriptFile {
        match self.editor.owner {
            None => &self.editor.file,
            Some(_) => self
                .editors
                .get(&None)
                .map_or(&self.editor.file, |editor| &editor.file),
        }
    }

    pub(super) fn saved_elsewhere(&mut self, path: &str, text: &str) {
        let owner = self.editor.owner;
        for editor in self.editors.values_mut().filter(|e| e.owner != owner) {
            let file = &mut editor.file;
            if file.path == path && file.disk.as_ref() == Some(&file.text) {
                file.text = text.to_owned();
                file.disk = Some(text.to_owned());
            }
        }
    }

    fn global_editor_mut(&mut self) -> &mut Editor {
        match self.editor.owner {
            None => &mut self.editor,
            Some(_) => self.editors.entry(None).or_default(),
        }
    }

    fn rescue_edits(&mut self, editor: Editor) {
        let (Some(pid), file) = (editor.owner, editor.file) else {
            return;
        };
        if !has_edits(&file) || file.text == self.global_script().text {
            return;
        }
        let name = file.name();
        if has_edits(self.global_script()) {
            self.error(format!(
                "Client {pid} closed with unsaved edits to {name}. They were lost: the editor has unsaved edits of its own."
            ));
        } else {
            self.global_editor_mut().file = file;
            self.info(format!(
                "Client {pid} closed with unsaved edits to {name}. They are in the editor now."
            ));
        }
    }

    pub(super) fn forget_gone_clients(&mut self) {
        let listed = self.listed_clients();
        if self.editor.owner.is_some_and(|pid| !listed.contains(&pid)) {
            self.show_editor(None);
        }
        let gone: Vec<Option<u32>> = self
            .editors
            .keys()
            .filter(|owner| owner.is_some_and(|pid| !listed.contains(&pid)))
            .copied()
            .collect();
        for owner in gone {
            if let Some(editor) = self.editors.remove(&owner) {
                self.rescue_edits(editor);
            }
        }
        self.parked_dev.retain(|pid, _| listed.contains(pid));
        if self.dev.pid.is_some_and(|pid| !listed.contains(&pid)) {
            if let Some(tiles) = self.dev.tiles.take() {
                tiles.finish();
            }
            self.dev = Inspectors::new(None);
        }
        if self.focus_client.is_some_and(|pid| !listed.contains(&pid)) {
            self.focus_client = None;
        }
    }
}

fn has_edits(file: &ScriptFile) -> bool {
    match &file.disk {
        Some(disk) => disk != &file.text,
        None => !file.text.trim().is_empty() && file.text != EXAMPLE_SCRIPT,
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;

    use super::super::sessions::{fake_run, running};
    use super::*;

    fn app(cx: &mut TestAppContext) -> LoaderApp {
        cx.update(|cx| {
            let mut app = LoaderApp::new(
                crate::storage::Storage::at(None),
                Config::from_env(),
                update::Instance::none(),
                Wake::default(),
                None,
                Chrome::new(cx),
            );
            super::super::clients::populated(&mut app);
            app
        })
    }

    #[gpui_kit::test]
    fn a_client_offers_pages_by_its_state(cx: &mut TestAppContext) {
        let mut app = app(cx);
        for page in [
            Page::Store(4120),
            Page::Queue(4120),
            Page::Editor(Some(4120)),
        ] {
            assert!(app.page_shown(page), "{page:?} on an idle loaded client");
        }
        assert!(!app.page_shown(Page::Development(5388)));
        assert!(!app.page_shown(Page::History(4120)));
        fake_run(&mut app, 4120, "Miner", running(false), None, 3);
        assert_eq!(app.client_state(4120), ClientState::Running);
        assert!(!app.page_shown(Page::Store(4120)));
        assert!(!app.page_shown(Page::Editor(Some(4120))));
        assert!(app.page_shown(Page::Development(4120)));
        app.page = Page::Store(4120);
        app.repair_page();
        assert_eq!(app.page, Page::Client(4120));
        if let Some(script) = app.runs[0].script.as_mut() {
            script.finish_now();
        }
        app.runs[0].finished = Some((Err("boom".into()), false));
        assert_eq!(app.client_state(4120), ClientState::Failed);
        assert!(app.page_shown(Page::History(4120)));
        assert!(app.page_shown(Page::Store(4120)));
        assert_eq!(app.client_state(5388), ClientState::Unloaded);
    }

    #[gpui_kit::test]
    fn an_unload_frees_the_buttons_while_its_rescan_runs(cx: &mut TestAppContext) {
        let mut app = app(cx);
        app.busy = true;
        app.tx
            .send(Event::Unloaded {
                pid: 4120,
                result: Ok(()),
            })
            .unwrap();
        app.handle_events();
        assert!(!app.busy);
        assert!(app.scanning);
        app.tx.send(Event::Scanned(Scan::default())).unwrap();
        app.handle_events();
        assert!(!app.busy && !app.scanning);
    }

    #[gpui_kit::test]
    fn a_closed_client_hands_unsaved_edits_to_the_global_editor(cx: &mut TestAppContext) {
        let mut app = app(cx);
        app.editor.file = ScriptFile {
            path: "/tmp/a.luau".into(),
            text: "print(1)".into(),
            disk: Some("print(1)".into()),
        };
        app.show_editor(Some(4120));
        app.editor.file.text = "print(2)".into();
        app.show_editor(Some(6012));
        app.targets.retain(|t| t.client.pid != 4120);
        app.forget_gone_clients();
        assert_eq!(app.global_script().text, "print(2)");
        assert_eq!(app.editor.owner, Some(6012));
        assert!(!app.editors.contains_key(&Some(4120)));
    }
}
