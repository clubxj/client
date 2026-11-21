use super::*;

impl LoaderApp {
    pub(super) fn render_terminal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.terminal.is_none() {
            let dir = crate::workspace::prepare(&self.settings)
                .map_err(|error| {
                    self.error(format!("Couldn't prepare the agent workspace: {error}"))
                })
                .ok();
            self.terminal = Some(crate::terminal::Terminal::new(self.wake.clone(), dir));
        }
        match &mut self.terminal {
            Some(terminal) => terminal.render(|app: &mut Self| app.terminal.as_mut(), window, cx),
            None => div().into_any_element(),
        }
    }
}

#[cfg(test)]
const SAMPLE: &str = concat!(
    "\x1b[32mvolcren@loader\x1b[0m:\x1b[34m~/agent\x1b[0m$ ls\r\n",
    "\x1b[1;34mlib\x1b[0m  \x1b[1;34mscripts\x1b[0m  CLAUDE.md  \x1b[32mrun.sh\x1b[0m\r\n",
    "\x1b[1mbold\x1b[0m \x1b[2mdim\x1b[0m \x1b[4munderline\x1b[0m \x1b[9mstrike\x1b[0m \x1b[7minverse\x1b[0m\r\n",
    "\x1b[30m0\x1b[31m1\x1b[32m2\x1b[33m3\x1b[34m4\x1b[35m5\x1b[36m6\x1b[37m7",
    "\x1b[90m8\x1b[91m9\x1b[92mA\x1b[93mB\x1b[94mC\x1b[95mD\x1b[96mE\x1b[97mF\x1b[0m ",
    "\x1b[41m red \x1b[0m \x1b[44;30m blue \x1b[0m \x1b[38;5;208m256\x1b[0m ",
    "\x1b[38;2;255;0;255mtruecolor\x1b[0m -> != wide: \u{6f22}\u{5b57}|\r\n",
    "\x1b[32mvolcren@loader\x1b[0m:\x1b[34m~/agent\x1b[0m$ echo ok",
);

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("terminal", |app| {
            app.tab = Tab::Terminal;
            app.terminal = Some(crate::terminal::Terminal::showing(SAMPLE.as_bytes(), false));
        }),
        ("terminal-exited", |app| {
            app.tab = Tab::Terminal;
            app.terminal = Some(crate::terminal::Terminal::showing(
                b"$ exit\r\nexit\r\n",
                true,
            ));
        }),
        ("terminal-error", |app| {
            app.tab = Tab::Terminal;
            app.terminal = Some(crate::terminal::Terminal::failed(
                "/bin/zsh: No such file or directory",
            ));
        }),
    ]
}
