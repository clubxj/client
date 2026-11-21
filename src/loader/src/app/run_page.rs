use std::rc::Rc;

use gpui_kit::component::scroll::ScrollableElement;

use super::*;
use crate::layouts::{Editing, Kind, Layout};
use crate::ui::{fields, theme, widgets};

#[derive(Default)]
pub(super) struct Ui {
    run: Option<u64>,
    form: Option<crate::form::FormUi>,
    layouts: crate::layouts::Ui,
    breaks: Option<crate::breaks::Ui>,
}

fn state_color(state: RunState) -> Rgba {
    match state {
        RunState::Running => theme::GREEN,
        RunState::Waiting => theme::BLUE,
        RunState::Paused => theme::YELLOW,
        RunState::Stopping => theme::RED,
        RunState::Ended => theme::COMMENT,
    }
}

fn page_width(window: &Window) -> Pixels {
    let width = window.viewport_size().width;
    let margin = (width * 0.02).clamp(px(8.0), px(16.0));
    let side = if width < px(SIDEBAR_MIN_WINDOW) {
        px(SIDEBAR_HANDLE_WIDTH)
    } else {
        px(SIDEBAR_WIDTH)
    };
    width - side - margin * 2.0
}

fn layout_with<L: Layout>(
    cx: &mut Context<LoaderApp>,
    run_id: u64,
    title: String,
) -> crate::layouts::With<L> {
    let app = cx.entity().downgrade();
    Rc::new(move |cx, change| {
        app.update(cx, |app, cx| {
            if let Some(run) = run_in(&mut app.runs, Some(run_id)) {
                let saved = L::saved_in(app.layouts.entry(title.clone()).or_default());
                change(L::editor_in(&mut run.editors), saved);
            }
            cx.notify();
        })
        .ok();
    })
}

fn layout_read(cx: &mut Context<LoaderApp>, kind: Kind) -> crate::layouts::OnRead {
    let app = cx.entity().downgrade();
    Rc::new(move |action, cx| {
        app.update(cx, |app, cx| {
            app.read_layout(kind, action == crate::layouts::Action::ReadQuick);
            cx.notify();
        })
        .ok();
    })
}

const TOOLS_TABS: [(ToolsTab, &str); 7] = [
    (ToolsTab::Settings, "Settings"),
    (ToolsTab::Equipment, "Equipment"),
    (ToolsTab::Inventory, "Inventory"),
    (ToolsTab::Prayer, "Prayer"),
    (ToolsTab::Xp, "Stats"),
    (ToolsTab::Breaks, "Breaks"),
    (ToolsTab::Feedback, "Feedback"),
];

impl LoaderApp {
    pub(super) fn render_run(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(run) = self.run() else {
            return div().into_any_element();
        };
        let running = run.is_running();
        let paused = running && run.is_paused();
        let stopping = run.stop_requested();
        let is_store = run.store.is_some();
        let waiting = run.is_configuring();
        let form_sent = !waiting && run.form_sent.is_some();
        let can_start = waiting && !self.start_pending();
        let changed_form = form_sent && run.form.as_ref().is_some_and(|f| {
            matches!(f.submitted(self.script_layouts(), &self.known_items), crate::form::Outcome::Submitted(v) if Some(&v) != run.form_sent.as_ref())
        });
        let submit_label = run
            .form
            .as_ref()
            .map_or("Start", |f| f.submit_label())
            .to_owned();
        let has = |kind: Kind| run.form.as_ref().is_some_and(|f| f.has_layout(kind));
        let shown = |tab: ToolsTab| match tab {
            ToolsTab::Equipment => has(Kind::Equipment),
            ToolsTab::Inventory => has(Kind::Inventory),
            ToolsTab::Prayer => has(Kind::Prayer),
            ToolsTab::Feedback => is_store,
            ToolsTab::Settings | ToolsTab::Xp | ToolsTab::Breaks => true,
        };
        let tabs: Vec<(ToolsTab, &str)> = TOOLS_TABS
            .into_iter()
            .filter(|&(tab, _)| shown(tab))
            .collect();
        let mut tools_tab = run.tools_tab;
        if !shown(tools_tab) {
            tools_tab = ToolsTab::Settings;
        }
        let run_id = run.id;
        let title = run.title.clone();
        let restartable = !running && !waiting;
        let restart_refusal = self.restart_refusal(run);
        let state = run.state();
        if self.run_ui.run != Some(run_id) {
            self.run_ui = Ui {
                run: Some(run_id),
                breaks: self.run_ui.breaks.take(),
                ..Default::default()
            };
        }
        if let Some(run) = self.run_mut() {
            run.tools_tab = tools_tab;
        }
        let queue = self.queue_banner(run_id);
        let width = widgets::icon_button_width(
            window,
            &[
                (ICON_START, &submit_label),
                (ICON_UPDATE, "Update"),
                PAUSE_BUTTONS[0],
                PAUSE_BUTTONS[1],
                (ICON_STOP, "Stop"),
                (ICON_CANCEL, "Cancel"),
                (ICON_RESTART, "Restart"),
            ],
        );
        let buttons = 3 + usize::from(restartable);
        let stack = (width + px(1.0)) * buttons as f32 + px(1.0) > page_width(window);
        let first = if form_sent {
            widgets::icon_button(
                "run-update",
                ICON_UPDATE,
                "Update",
                width,
                running && !stopping && changed_form,
                cx.listener(|app, _: &ClickEvent, _, cx| {
                    app.push_form_update();
                    cx.notify();
                }),
            )
        } else {
            widgets::icon_button(
                "run-start",
                ICON_START,
                submit_label,
                width,
                can_start,
                cx.listener(move |app, _: &ClickEvent, _, cx| {
                    let outcome = app.run().and_then(|r| r.form.as_ref()).map_or_else(
                        || crate::form::Outcome::Submitted(serde_json::json!({})),
                        |form| form.submitted(app.script_layouts(), &app.known_items),
                    );
                    app.answer_form(run_id, outcome);
                    cx.notify();
                }),
            )
        };
        let (pause_icon, pause_text) = run_pause_label(paused);
        let can_pause = running && !stopping && !waiting;
        let pause = widgets::icon_button(
            "run-pause",
            pause_icon,
            pause_text,
            width,
            can_pause,
            cx.listener(|app, _: &ClickEvent, _, cx| {
                if let Some(id) = app.active_run {
                    app.toggle_pause_of(id);
                }
                cx.notify();
            }),
        )
        .when(is_store && can_pause, |this| {
            this.tooltip(widgets::tooltip(
                "Still billed while paused: stop the script to stop billing.",
            ))
        });
        let (stop_icon, stop_text) = if waiting {
            (ICON_CANCEL, "Cancel")
        } else {
            (ICON_STOP, "Stop")
        };
        let stop = widgets::icon_button(
            "run-stop",
            stop_icon,
            stop_text,
            width,
            waiting || (running && !stopping),
            cx.listener(move |app, _: &ClickEvent, _, cx| {
                if waiting {
                    app.answer_form(run_id, crate::form::Outcome::Cancelled);
                } else if let Some(run) = app.run_mut() {
                    run.stop();
                }
                cx.notify();
            }),
        );
        let restart = restartable.then(|| {
            widgets::icon_button(
                "run-restart",
                ICON_RESTART,
                "Restart",
                width,
                restart_refusal.is_none(),
                cx.listener(move |app, _: &ClickEvent, _, cx| {
                    app.restart_run(run_id);
                    cx.notify();
                }),
            )
            .when_some(restart_refusal, |this, why| {
                this.tooltip(widgets::tooltip(why))
            })
        });
        let group = widgets::button_group(stack)
            .child(first)
            .child(pause)
            .child(stop)
            .children(restart);
        let strip = tabs
            .iter()
            .fold(widgets::tab_strip(), |strip, &(tab, label)| {
                strip.child(widgets::tab_value(
                    ("tools-tab", tab as usize),
                    label,
                    tab == tools_tab,
                    cx.listener(move |app, _: &ClickEvent, _, cx| {
                        if let Some(run) = run_in(&mut app.runs, Some(run_id)) {
                            run.tools_tab = tab;
                        }
                        cx.notify();
                    }),
                ))
            });
        let content = self.render_tools_tab(tools_tab, run_id, window, cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .mt(px(4.0))
                    .w_full()
                    .min_w_0()
                    .child(widgets::heading(title).truncate()),
            )
            .children(queue.map(|progress| {
                div()
                    .flex_none()
                    .mt(theme::LINE_GAP)
                    .text_color(theme::BLUE)
                    .child(format!("Queue {progress}"))
            }))
            .child(div().flex().flex_none().mt(px(12.0)).child(group))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .mt(px(8.0))
                    .gap(theme::ITEM_GAP)
                    .child(widgets::status_dot(state_color(state)))
                    .child(div().text_color(state_color(state)).child(state.label())),
            )
            .child(div().flex_none().mt(px(20.0)).child(strip))
            .child(divider())
            .child(
                div().flex_1().min_h_0().mt(px(8.0)).child(
                    div()
                        .flex()
                        .flex_col()
                        .child(widgets::tab_content().child(content))
                        .overflow_y_scrollbar()
                        .id(("tools-scroll", tools_tab as usize)),
                ),
            )
            .into_any_element()
    }

    fn render_tools_tab(
        &mut self,
        tools_tab: ToolsTab,
        run_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let can_read = self.layout_target().is_some();
        let form_title = self
            .run()
            .and_then(|r| r.form.as_ref())
            .map(|form| form.title.clone())
            .unwrap_or_default();
        let unknown: Vec<i32> = self
            .run()
            .and_then(|r| r.form.as_ref())
            .map(|form| {
                [Kind::Equipment, Kind::Inventory, Kind::Prayer]
                    .into_iter()
                    .flat_map(|kind| form.rules(kind).unknown(&self.known_items))
                    .filter(|id| !self.asking_items.contains(id))
                    .collect()
            })
            .unwrap_or_default();
        let content = match tools_tab {
            ToolsTab::Settings => self.render_run_settings(run_id, window, cx),
            ToolsTab::Equipment => {
                let with = layout_with(cx, run_id, form_title.clone());
                let on_read = layout_read(cx, Kind::Equipment);
                let saved = self.layouts.entry(form_title).or_default();
                match run_in(&mut self.runs, self.active_run) {
                    Some(Run { editors, form, .. }) => crate::layouts::equipment_ui(
                        Editing {
                            editor: &mut editors.equipment,
                            saved: &mut saved.equipment,
                            rules: rules_in(form, Kind::Equipment),
                            can_read,
                            with,
                            on_read,
                        },
                        &mut self.run_ui.layouts,
                        &mut self.icons,
                        &self.known_items,
                        window,
                        cx,
                    )
                    .into_any_element(),
                    None => div().into_any_element(),
                }
            }
            ToolsTab::Inventory => {
                let with = layout_with(cx, run_id, form_title.clone());
                let on_read = layout_read(cx, Kind::Inventory);
                let saved = self.layouts.entry(form_title).or_default();
                match run_in(&mut self.runs, self.active_run) {
                    Some(Run { editors, form, .. }) => crate::layouts::inventory_ui(
                        Editing {
                            editor: &mut editors.inventory,
                            saved: &mut saved.inventory,
                            rules: rules_in(form, Kind::Inventory),
                            can_read,
                            with,
                            on_read,
                        },
                        &mut self.run_ui.layouts,
                        &mut self.icons,
                        &self.known_items,
                        window,
                        cx,
                    )
                    .into_any_element(),
                    None => div().into_any_element(),
                }
            }
            ToolsTab::Prayer => {
                let level = self
                    .run()
                    .and_then(|r| r.xp.rows().into_iter().find(|row| row.name == "Prayer"))
                    .and_then(|row| u32::try_from(row.level).ok());
                let with = layout_with(cx, run_id, form_title.clone());
                let on_read = layout_read(cx, Kind::Prayer);
                let saved = self.layouts.entry(form_title).or_default();
                match run_in(&mut self.runs, self.active_run) {
                    Some(Run { editors, form, .. }) => crate::layouts::prayer_ui(
                        Editing {
                            editor: &mut editors.prayer,
                            saved: &mut saved.prayer,
                            rules: rules_in(form, Kind::Prayer),
                            can_read,
                            with,
                            on_read,
                        },
                        &mut self.run_ui.layouts,
                        &mut self.icons,
                        level,
                        window,
                        cx,
                    )
                    .into_any_element(),
                    None => div().into_any_element(),
                }
            }
            ToolsTab::Xp => self.render_stats().into_any_element(),
            ToolsTab::Breaks => self.render_breaks(window, cx).into_any_element(),
            ToolsTab::Feedback => self.render_run_feedback(),
        };
        if !unknown.is_empty() {
            self.describe_items(unknown);
        }
        content
    }

    fn render_run_settings(
        &mut self,
        run_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = cx.entity().downgrade();
        let Some(form) = run_in(&mut self.runs, self.active_run).and_then(|r| r.form.as_mut())
        else {
            return widgets::weak("The script isn't asking for settings.").into_any_element();
        };
        let layouts = self.layouts.get(&form.title).unwrap_or(&EMPTY_LAYOUTS);
        let ui = self.run_ui.form.get_or_insert_with(|| {
            crate::form::FormUi::new(move |cx, change| {
                app.update(cx, |app, cx| {
                    if let Some(form) =
                        run_in(&mut app.runs, Some(run_id)).and_then(|r| r.form.as_mut())
                    {
                        change(form);
                    }
                    cx.notify();
                })
                .ok();
            })
        });
        form.render(ui, layouts, window, cx)
    }

    fn render_stats(&self) -> Div {
        use crate::{
            counters::number,
            xp::{duration, thousands},
        };
        let Some(run) = self
            .run()
            .filter(|r| !(r.xp.is_empty() && r.counters.is_empty()))
        else {
            return widgets::weak("Waiting for the first read of the client's skills.");
        };
        let rows = run.xp.rows();
        let grid = |columns: u16| {
            div()
                .grid()
                .grid_cols_max_content(columns)
                .gap_x(px(12.0))
                .gap_y(px(2.0))
        };
        let skills = if rows.is_empty() {
            widgets::weak("No experience gained yet.")
        } else {
            let heads = ["Skill", "Gained", "XP/hr", "Level", "To level"]
                .into_iter()
                .fold(grid(5), |grid, head| grid.child(widgets::weak(head)));
            rows.iter().fold(heads, |grid, row| {
                grid.child(row.name.clone())
                    .child(thousands(row.gained))
                    .child(row.per_hour.map_or_else(|| "-".into(), thousands))
                    .child(row.level.to_string())
                    .child(row.to_level.map_or_else(|| "-".into(), duration))
            })
        };
        let counters = run.counters.rows();
        let counters = (!counters.is_empty()).then(|| {
            let heads = ["", "Total", "Per hour", ""]
                .into_iter()
                .fold(grid(4), |grid, head| grid.child(widgets::weak(head)));
            let table = counters.iter().fold(heads, |grid, row| {
                grid.child(row.label.clone())
                    .child(number(row.total))
                    .child(row.per_hour.map_or_else(|| "-".into(), number))
                    .child(row.ratio.clone().unwrap_or_default())
            });
            div()
                .mt(px(6.0))
                .flex()
                .flex_col()
                .child(widgets::separator())
                .child(table)
        });
        div()
            .flex()
            .flex_col()
            .gap(theme::LINE_GAP)
            .child(format!(
                "{} · {} xp",
                duration(run.run_time()),
                thousands(run.xp.total())
            ))
            .child(skills)
            .children(counters)
    }

    fn render_breaks(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        use crate::{breaks::clock, xp::duration};
        let running = self.run().is_some_and(Run::is_running);
        let app = cx.entity().downgrade();
        let ui = self.run_ui.breaks.get_or_insert_with(|| {
            crate::breaks::Ui::new(move |cx, change| {
                app.update(cx, |app, cx| {
                    change(&mut app.settings.antiban);
                    cx.notify();
                })
                .ok();
            })
        });
        let sleeps = self.breaks_tab == BreaksTab::Sleeps;
        let tab = |tab: BreaksTab, label: &'static str| {
            widgets::tab_value(
                ("breaks-tab", tab as usize),
                label,
                self.breaks_tab == tab,
                cx.listener(move |app, _: &ClickEvent, _, cx| {
                    app.breaks_tab = tab;
                    cx.notify();
                }),
            )
        };
        let tabs = widgets::tab_strip()
            .child(tab(BreaksTab::Breaks, "Breaks"))
            .child(tab(BreaksTab::Sleeps, "Sleeps"));
        let editor = if sleeps {
            crate::breaks::sleeps_editor(&mut self.settings.antiban, ui, window, cx)
        } else {
            crate::breaks::breaks_editor(&mut self.settings.antiban, ui, window, cx)
        };
        let page = div()
            .flex()
            .flex_col()
            .gap(theme::LINE_GAP)
            .child(crate::breaks::header())
            .child(
                div()
                    .mt(theme::LINE_GAP)
                    .flex()
                    .flex_col()
                    .child(tabs)
                    .child(widgets::tab_panel().child(editor)),
            );
        let valid = self.settings.antiban.validate();
        let page = page.when_some(valid.as_ref().err(), |this, why| {
            this.child(widgets::error(why.clone()).mt(px(6.0)))
        });
        if !running {
            return page.child(
                widgets::weak("Used by the scripts you start. It is saved as you edit.")
                    .mt(px(6.0)),
            );
        }
        let dirty = self
            .run()
            .is_some_and(|r| r.antiban_sent != self.settings.antiban);
        let can_update = dirty && valid.is_ok();
        let update = widgets::button(
            "breaks-update",
            "Update",
            can_update,
            cx.listener(|app, _: &ClickEvent, _, cx| {
                let config = app.settings.antiban.clone();
                if let Some(run) = app.run_mut() {
                    match run.send(|s| s.push_antiban(&config)) {
                        Ok(()) => {
                            run.antiban_sent = config;
                            run.info("Breaks updated.");
                        }
                        Err(error) => run.error(error),
                    }
                }
                cx.notify();
            }),
        )
        .when(can_update, |this| {
            this.tooltip(widgets::tooltip("Send these to the running script"))
        });
        let page = page.child(
            div()
                .mt(px(6.0))
                .mb(px(6.0))
                .flex()
                .items_center()
                .gap(theme::ITEM_GAP)
                .child(update)
                .when(dirty, |this| {
                    this.child(widgets::weak("Not sent to the script yet"))
                }),
        );
        let Some(view) = self.run().and_then(|r| r.breaks.view(Instant::now())) else {
            return page
                .child(widgets::weak("The script hasn't reported any breaks yet.").mt(px(6.0)));
        };
        self.wake.wake_after(Duration::from_secs(1));
        let skip = |kind: &'static str, index: u32, row: usize| {
            fields::small_button(
                (kind, row),
                "Skip next",
                true,
                cx.listener(move |app, _: &ClickEvent, _, cx| {
                    if let Some(run) = app.run_mut() {
                        let result = run.send(|s| s.skip_next(kind, index));
                        run.report(result, &format!("Skipping the next {kind}."));
                    }
                    cx.notify();
                }),
            )
        };
        let grid = |columns: u16| {
            div()
                .mt(theme::LINE_GAP)
                .grid()
                .grid_cols_max_content(columns)
                .items_center()
                .gap_x(px(12.0))
                .gap_y(px(2.0))
        };
        let queued = if sleeps {
            (!view.sleeps.is_empty()).then(|| {
                view.sleeps
                    .iter()
                    .enumerate()
                    .fold(grid(4), |grid, (i, row)| {
                        grid.child(row.at.map_or_else(|| "-".into(), clock))
                            .child(format!("in {}", duration(row.remaining)))
                            .child(format!("~{}", duration(row.length)))
                            .child(skip("sleep", row.index, i))
                    })
            })
        } else {
            (!view.breaks.is_empty()).then(|| {
                view.breaks
                    .iter()
                    .enumerate()
                    .fold(grid(3), |grid, (i, row)| {
                        grid.child(format!("in {}", duration(row.remaining)))
                            .child(format!("~{}", duration(row.length)))
                            .child(skip("break", row.index, i))
                    })
            })
        };
        let nothing = queued.is_none();
        page.child(widgets::separator())
            .child(widgets::strong("Queued").mt(theme::LINE_GAP))
            .children(view.active.as_ref().map(|active| {
                widgets::strong(format!(
                    "On a {} · {} left",
                    active.kind,
                    duration(active.remaining)
                ))
            }))
            .children(queued)
            .when(nothing, |this| {
                this.child(widgets::weak("Nothing is scheduled."))
            })
    }

    fn render_run_feedback(&mut self) -> AnyElement {
        let Some(session_id) = self.run().and_then(|r| r.session_id().map(str::to_owned)) else {
            return div().into_any_element();
        };
        self.fetch_sessions_if_stale();
        if !self
            .feedback
            .sessions
            .iter()
            .any(|s| s.session_id == session_id)
        {
            return div()
                .flex()
                .flex_col()
                .gap(theme::LINE_GAP)
                .child(widgets::weak(if self.feedback.loading {
                    "Loading…"
                } else {
                    "This run isn't listed yet; it is once the server has it."
                }))
                .children(self.feedback.error.clone().map(error_label))
                .into_any_element();
        }
        not_ported("Run feedback")
    }
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("run-configuring", |app| {
            fakes::add_run(app, None, true);
        }),
        ("run-equipment", |app| {
            let id = fakes::running(app, ToolsTab::Equipment);
            fakes::open_layouts(app, id);
        }),
        ("run-inventory", |app| {
            let id = fakes::running(app, ToolsTab::Inventory);
            fakes::open_layouts(app, id);
        }),
        ("run-prayer", |app| {
            let id = fakes::running(app, ToolsTab::Prayer);
            fakes::open_layouts(app, id);
        }),
        ("run-new-layout", |app| {
            fakes::running(app, ToolsTab::Inventory);
        }),
        ("run-stats", |app| {
            let id = fakes::running(app, ToolsTab::Xp);
            let run = run_in(&mut app.runs, Some(id)).expect("just added");
            let skill = |name: &str, level, xp| crate::xp::SkillXp {
                name: name.into(),
                level,
                xp,
            };
            run.xp.update(
                vec![
                    skill("Fishing", 76, 1_336_443),
                    skill("Prayer", 70, 737_627),
                ],
                Duration::ZERO,
            );
            run.xp.update(
                vec![
                    skill("Fishing", 77, 1_480_000),
                    skill("Prayer", 70, 739_000),
                ],
                Duration::from_secs(5400),
            );
            run.counters.update(
                vec![
                    injector::form::Counter {
                        id: "fish".into(),
                        label: "Fish".into(),
                        value: 1210.0,
                        hourly: true,
                        per: None,
                        percent_of: None,
                    },
                    injector::form::Counter {
                        id: "trips".into(),
                        label: "Trips".into(),
                        value: 44.0,
                        hourly: true,
                        per: None,
                        percent_of: None,
                    },
                    injector::form::Counter {
                        id: "per_trip".into(),
                        label: "Fish per trip".into(),
                        value: 1210.0,
                        hourly: false,
                        per: Some("trips".into()),
                        percent_of: None,
                    },
                ],
                Duration::from_secs(5400),
            );
        }),
        ("run-breaks", |app| {
            let id = fakes::running(app, ToolsTab::Breaks);
            app.settings.antiban.breaks[0].randomness = Some(0.25);
            let run = run_in(&mut app.runs, Some(id)).expect("just added");
            run.breaks.update(
                injector::form::BreakPlan {
                    breaks: vec![
                        injector::form::PlannedBreak {
                            remaining: 754_000.0,
                            length: 300_000.0,
                            at: None,
                            index: 0,
                        },
                        injector::form::PlannedBreak {
                            remaining: 4_100_000.0,
                            length: 900_000.0,
                            at: None,
                            index: 1,
                        },
                    ],
                    sleeps: vec![],
                    active: None,
                },
                Instant::now(),
            );
        }),
        ("run-sleeps", |app| {
            fakes::running(app, ToolsTab::Breaks);
            app.breaks_tab = BreaksTab::Sleeps;
            if let Some(sleep) = app.settings.antiban.sleeps.first_mut() {
                sleep.time = Some("25:00".into());
                sleep.length = Some(7.5 * 3_600_000.0);
            }
        }),
        ("run-paused", |app| {
            let id = fakes::add_run(app, Some(Script::idle(true)), true);
            if let Some(run) = run_in(&mut app.runs, Some(id)) {
                run.tools_tab = ToolsTab::Feedback;
            }
        }),
        ("run-ended-queue", |app| {
            let id = fakes::add_run(app, Some(Script::idle(false)), true);
            if let Some(script) = run_in(&mut app.runs, Some(id)).and_then(|r| r.script.as_mut()) {
                script.finish_now();
            }
            let queue = crate::queue::Queue {
                name: "Evening".into(),
                entries: vec![crate::queue::Entry {
                    script_id: "snapshot-script".into(),
                    title: "Snapshot script".into(),
                    settings: Default::default(),
                    spec_hash: None,
                    goal: None,
                    min_minutes: None,
                    max_minutes: Some(90),
                }],
                ..Default::default()
            };
            let mut runner = crate::queue::Runner::new(queue, 1, Default::default());
            runner.phase = crate::queue::Phase::Running {
                run: id,
                finish_sent: None,
            };
            app.queue_runners.push(runner);
        }),
    ]
}

#[cfg(test)]
mod fakes {
    use super::*;

    pub(super) fn form_spec() -> serde_json::Value {
        serde_json::json!({
            "title": "Snapshot script",
            "submit": "Start fishing",
            "fields": [
                { "kind": "label", "text": "Trip", "heading": 1 },
                { "kind": "label", "text": "Pick what the script does on each trip." },
                { "kind": "choice", "id": "spot", "label": "Fishing spot", "options": ["Barbarian", "Karambwan", "Shilo"], "default": "Karambwan" },
                { "kind": "number", "id": "trips", "label": "Trips", "min": 1, "max": 500, "integer": true, "default": 20, "row": "limits" },
                { "kind": "number", "id": "eat_at", "label": "% health", "min": 0, "max": 100, "step": 0.5, "default": 42.5, "box_first": true, "row": "limits" },
                { "kind": "text", "id": "note", "label": "Note", "default": "bank at the deposit box" },
                { "kind": "label", "text": "Safety", "heading": 2 },
                { "kind": "check", "id": "hop", "label": "Hop worlds when a player is near", "default": true },
                { "kind": "check", "id": "logout", "label": "Log out when out of food", "enabled_by": "hop" },
                { "kind": "check", "id": "panic", "label": "Teleport away under 20% health", "disabled_by": "hop" },
                { "kind": "list", "id": "fish", "label": "Keep", "items": [
                    { "id": "raw", "label": "Raw fish", "default": true },
                    { "id": "roe", "label": "Roe" },
                    { "id": "caviar", "label": "Caviar", "default": true },
                    { "id": "clue", "label": "Clue bottles", "default": true }
                ] },
                { "kind": "label", "text": "Layouts", "heading": 2 },
                { "kind": "equipment", "id": "gear", "label": "Gear", "required": [{ "ids": [11323], "slot": 3 }] },
                { "kind": "inventory", "id": "bag", "required": [{ "ids": [2434], "slot": 0, "doses": [4, 3] }, { "ids": [314], "min": 100, "max": 0 }] },
                { "kind": "prayer", "id": "prayers", "locked": ["protect_from_melee"] }
            ]
        })
    }

    pub(super) fn known() -> crate::layouts::Known {
        crate::layouts::Known::from([
            (11323, ("Barbarian rod".to_owned(), false)),
            (2434, ("Prayer potion(4)".to_owned(), false)),
            (314, ("Feather".to_owned(), true)),
            (385, ("Shark".to_owned(), false)),
            (12625, ("Stamina potion(4)".to_owned(), false)),
            (12791, ("Rune pouch".to_owned(), false)),
            (556, ("Air rune".to_owned(), true)),
            (1075, ("Iron platelegs".to_owned(), false)),
            (1153, ("Iron full helm".to_owned(), false)),
            (9244, ("Dragonstone dragon bolts (e)".to_owned(), true)),
        ])
    }

    fn item(id: i32, quantity: i32) -> crate::layouts::Item {
        let (name, stackable) = known().get(&id).cloned().unwrap_or_default();
        crate::layouts::Item::read(id, quantity, name, stackable)
    }

    pub(super) fn layouts() -> crate::layouts::ScriptLayouts {
        use crate::layouts::{Doses, Equipment, EquipmentSlot, Inventory, Prayer, ScriptLayouts};
        let mut slots = vec![EquipmentSlot::Any; crate::layouts::EQUIPMENT_SIZE];
        slots[0] = EquipmentSlot::Item(item(1153, 1));
        slots[7] = EquipmentSlot::Item(item(1075, 1));
        slots[13] = EquipmentSlot::Item(item(9244, 120));
        slots[12] = EquipmentSlot::Empty;
        let mut potion = item(12625, 1);
        potion.doses = Some(Doses {
            all: vec![1, 2, 3, 4],
            accepted: vec![4, 3],
            highest_first: true,
        });
        let mut bag = vec![None; crate::layouts::INVENTORY_SIZE];
        bag[1] = Some(item(12791, 1));
        bag[4] = Some(potion);
        bag[8..14].fill(Some(item(385, 1)));
        ScriptLayouts {
            equipment: vec![Equipment {
                name: "Fishing".into(),
                slots,
                spell: Some("fire_bolt".into()),
                autocast: true,
            }],
            inventory: vec![Inventory {
                name: "Trip".into(),
                slots: bag,
                pouch: Some(vec![item(556, 2000)]),
            }],
            prayer: vec![Prayer {
                name: "Boost".into(),
                prayers: vec!["eagle_eye".into(), "protect_item".into()],
            }],
        }
    }

    pub(super) fn add_run(app: &mut LoaderApp, script: Option<Script>, store: bool) -> u64 {
        let id = app.next_run_id();
        let store = store.then(|| StoreRun {
            session_id: "snapshot-session".into(),
            script_id: "snapshot-script".into(),
            community: false,
            closed: channel().1,
            lib_version: "1".into(),
            price: 2_000_000,
            rank: 0,
            fee: 0,
            fee_charged: 0,
        });
        let mut run = Run::new(
            id,
            "Snapshot script".into(),
            script,
            None,
            store,
            &app.settings,
        );
        let mut form = crate::form::Form::new(form_spec(), None);
        if run.script.is_some()
            && let crate::form::Outcome::Submitted(values) = form.submitted(&layouts(), &known())
        {
            run.form_sent = Some(values);
        }
        run.form = Some(std::mem::replace(
            &mut form,
            crate::form::Form::new(form_spec(), None),
        ));
        app.layouts.insert("Snapshot script".into(), layouts());
        app.known_items = known();
        app.feedback.fetched_at = Some(Instant::now());
        app.runs.push(run);
        app.active_run = Some(id);
        app.tab = Tab::Run;
        id
    }

    pub(super) fn running(app: &mut LoaderApp, tab: ToolsTab) -> u64 {
        let id = add_run(app, Some(Script::idle(false)), false);
        let run = run_in(&mut app.runs, Some(id)).expect("just added");
        run.tools_tab = tab;
        id
    }

    pub(super) fn open_layouts(app: &mut LoaderApp, id: u64) {
        let saved = layouts();
        let run = run_in(&mut app.runs, Some(id)).expect("just added");
        let editors = &mut run.editors;
        editors.equipment.open(&saved.equipment, Some(0));
        editors.equipment.selected = Some(13);
        editors.inventory.open(&saved.inventory, Some(0));
        editors.inventory.selected = Some(4);
        editors.prayer.open(&saved.prayer, Some(0));
        editors.prayer.selected = Some(18);
    }
}
