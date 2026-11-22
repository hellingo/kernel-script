use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use mlua::{Lua, Value};

mod config_api;
mod draw_api;
mod engine_api;
mod execution;
mod keyboard;
mod memory_api;
mod types;
pub(crate) mod ui_api;

use crate::config_store::SharedConfigStore;
use config_api::register_config_api;
use draw_api::register_draw_api;
use memory_api::register_memory_api;
use types::RuntimeControl;
pub use types::{DrawCommand, DrawCommands};
use ui_api::create_ui_module;

/// Refresh the overlay content scale used by `memory.get_window_rect`'s
/// fallback path (the target window's own monitor DPI is preferred).
pub fn set_content_scale(scale: f32) {
    crate::window_util::set_content_scale(scale);
}

// The overlay repaints at 100 Hz (see `app::FRAME_INTERVAL`). OnUpdate runs
// once per rendered frame, which keeps immediate-mode egui windows stable; a
// slow callback simply lowers the achieved frequency.
const MAX_FRAME_DELTA: Duration = Duration::from_millis(250);
const RELOAD_POLL_INTERVAL: Duration = Duration::from_millis(250);
const LUA_MEMORY_LIMIT: usize = 64 * 1024 * 1024;
const START_BUDGET: Duration = Duration::from_millis(100);
const SCRIPT_START_DELAY: Duration = Duration::from_millis(250);
const UPDATE_BUDGET: Duration = Duration::from_millis(20);
const DESTROY_BUDGET: Duration = Duration::from_millis(50);
const UPDATE_HEALTHY_FRAMES_TO_CLEAR_ERROR: u32 = 30;

pub struct LuaRuntime {
    lua: Lua,
    script_path: PathBuf,
    control: Arc<RuntimeControl>,
    deadline: Arc<AtomicU64>,
    last_error: Option<String>,
    consecutive_healthy_updates: u32,
    last_tick: Instant,
    last_reload_poll: Instant,
    script_modified: Option<SystemTime>,
    draw_commands: DrawCommands,
    config: SharedConfigStore,
    keyboard: keyboard::SharedKeyboardState,
}

/// Union of every egui window the Lua UI (and the host chrome) painted this
/// frame, in egui points. The overlay host uses it to decide where the
/// transparent window must accept the mouse instead of clicking through.
pub type UiRectHandle = std::rc::Rc<std::cell::Cell<Option<egui::Rect>>>;

pub struct LuaRuntimeManager {
    runtimes: Vec<LuaRuntime>,
    pending_scripts: VecDeque<PathBuf>,
    startup_errors: Vec<(PathBuf, String)>,
    script_load_after: Option<Instant>,
    draw_commands: DrawCommands,
    config: SharedConfigStore,
    ui_rect: UiRectHandle,
}

impl LuaRuntimeManager {
    pub fn new(script_directory: PathBuf) -> mlua::Result<Self> {
        let draw_commands: DrawCommands = Arc::new(Mutex::new(Vec::new()));
        // config.json lives beside the executable (scripts/<dir>/..). The
        // store is manager-owned so it survives per-script hot reloads.
        let config = crate::config_store::ConfigStore::shared(
            script_directory
                .parent()
                .unwrap_or(script_directory.as_path()),
        );
        // A missing or empty `scripts` directory is not an error: the
        // overlay and its control window run without Lua, and the separate
        // `ks-inspector` process is unaffected, so the GUI must start
        // regardless. `.lua` files whose
        // stem starts with `_` are treated as disabled.
        let mut paths = fs::read_dir(&script_directory)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "lua"))
            .filter(|path| {
                path.file_stem()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| !name.starts_with('_'))
            })
            .collect::<Vec<_>>();
        paths.sort();
        if paths.is_empty() {
            tracing::info!(
                directory = %script_directory.display(),
                "no Lua scripts loaded; the overlay and memory tool still run"
            );
        }
        Ok(Self {
            runtimes: Vec::new(),
            pending_scripts: paths.into(),
            startup_errors: Vec::new(),
            script_load_after: None,
            draw_commands,
            config,
            ui_rect: std::rc::Rc::new(std::cell::Cell::new(None)),
        })
    }

    /// The manager-owned config store (`config.json`), shared with the GUI
    /// chrome so the language choice persists beside script state.
    pub fn config_store(&self) -> SharedConfigStore {
        Arc::clone(&self.config)
    }

    /// Shared window-union handle (egui points). The display loop clears it
    /// at the start of every frame; `ui.window`, the host control window and
    /// the Lua error window union their rects into it.
    pub fn ui_rect_handle(&self) -> UiRectHandle {
        self.ui_rect.clone()
    }

    fn load_pending_script(&mut self, now: Instant) {
        let ready = *self
            .script_load_after
            .get_or_insert(now + SCRIPT_START_DELAY);
        if now < ready || crate::driver::lifecycle_busy() {
            return;
        }
        let Some(path) = self.pending_scripts.pop_front() else {
            return;
        };
        // Initialize one VM per frame on the GUI Lua thread after the first
        // frames are visible. A bad file must not close the entire overlay.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            LuaRuntime::new(
                path.clone(),
                Arc::clone(&self.draw_commands),
                Arc::clone(&self.config),
            )
        }));
        match result {
            Ok(Ok(runtime)) => self.runtimes.push(runtime),
            result => {
                let message = match result {
                    Ok(Err(error)) => error.to_string(),
                    _ => "Lua initialization panicked".to_owned(),
                };
                tracing::error!(script = %path.display(), error = %message, "Lua startup failed; GUI remains open");
                self.startup_errors.push((path, message));
            }
        }
    }

    fn retry_failed_scripts(&mut self) {
        self.pending_scripts
            .extend(self.startup_errors.drain(..).map(|(path, _)| path));
    }

    pub fn frame(&mut self, ctx: &egui::Context, now: Instant, ui_visible: bool) {
        // Every rendered frame runs OnUpdate (immediate-mode egui requires it)
        // and regenerates the draw command snapshot. The interactive-region
        // union is owned by the display loop: it must survive the host
        // control window drawn before this frame's Lua UI.
        self.draw_commands.lock().unwrap().clear();
        self.load_pending_script(now);
        let now = now.max(Instant::now());
        let ui_rect = self.ui_rect.clone();
        for runtime in &mut self.runtimes {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                runtime.frame(ctx, now, ui_visible, &ui_rect);
            }));
            if result.is_err() {
                runtime.set_error("Lua runtime panicked; script disabled until reload".to_owned());
            }
        }
        if ui_visible && !self.startup_errors.is_empty() {
            let mut retry = false;
            let response = egui::Window::new("Lua startup errors").show(ctx, |ui| {
                for (path, message) in &self.startup_errors {
                    ui.label(path.display().to_string());
                    ui.label(message);
                    ui.separator();
                }
                retry = ui.button("Retry failed scripts").clicked();
            });
            if let Some(response) = response {
                ui_api::record_ui_rect(&self.ui_rect, response.response.rect);
            }
            if retry {
                self.retry_failed_scripts();
            }
        }
        self.config.lock().unwrap().flush_if_due();
    }

    pub fn set_error(&mut self, error: String) {
        for runtime in &mut self.runtimes {
            runtime.set_error(error.clone());
        }
    }

    pub fn take_draw_commands(&self) -> Vec<DrawCommand> {
        self.draw_commands.lock().unwrap().drain(..).collect()
    }
}

impl LuaRuntime {
    fn new(
        script_path: PathBuf,
        draw_commands: DrawCommands,
        config: SharedConfigStore,
    ) -> mlua::Result<Self> {
        let control = Arc::new(RuntimeControl::default());
        let keyboard = Arc::new(Mutex::new(keyboard::KeyboardState::new()));
        let (lua, deadline) = Self::build_vm(
            &script_path,
            Arc::clone(&control),
            Arc::clone(&draw_commands),
            Arc::clone(&config),
            Arc::clone(&keyboard),
        )?;
        let now = Instant::now();
        Ok(Self {
            lua,
            script_modified: script_modified(&script_path),
            draw_commands,
            script_path,
            control,
            deadline,
            last_error: None,
            consecutive_healthy_updates: 0,
            last_tick: now,
            last_reload_poll: now,
            config,
            keyboard,
        })
    }

    fn frame(
        &mut self,
        ctx: &egui::Context,
        now: Instant,
        ui_visible: bool,
        ui_rect: &UiRectHandle,
    ) {
        self.check_hot_reload(now);
        // Snapshot all keys once per frame so is_key_press reports a stable
        // down-edge for the duration of OnUpdate.
        self.keyboard.lock().unwrap().begin_frame();
        self.run_updates(ctx, now, ui_visible, ui_rect);
        if let Err(error) = self.lua.gc_step() {
            self.last_error = Some(format!("Lua GC error: {error}"));
        }
    }

    pub fn set_error(&mut self, error: String) {
        self.last_error = Some(error);
    }

    fn build_vm(
        script_path: &Path,
        control: Arc<RuntimeControl>,
        draw_commands: DrawCommands,
        config: SharedConfigStore,
        keyboard: keyboard::SharedKeyboardState,
    ) -> mlua::Result<(Lua, Arc<AtomicU64>)> {
        let lua = Lua::new();
        lua.set_memory_limit(LUA_MEMORY_LIMIT)?;
        // Scripts only need the registered engine and memory APIs.
        for global in ["os", "io", "package", "debug"] {
            lua.globals().set(global, Value::Nil)?;
        }
        let deadline = execution::install_hook(&lua);
        engine_api::register(&lua, control)?;
        register_memory_api(&lua)?;
        register_draw_api(&lua, draw_commands)?;
        register_config_api(&lua, config)?;
        keyboard::register(&lua, keyboard)?;

        let source = fs::read_to_string(script_path).map_err(mlua::Error::external)?;
        // Compilation is finite host work; keep it out of the user-code
        // execution budget so slower CPUs can load large, ordinary Lua files.
        let chunk = lua
            .load(&source)
            .set_name(script_path.to_string_lossy().as_ref())
            .into_function()?;
        execution::set_deadline(&deadline, Some(START_BUDGET));
        let load_result = chunk.call::<()>(());
        execution::set_deadline(&deadline, None);
        load_result?;
        execution::call_budgeted(&lua, &deadline, "OnStart", (), START_BUDGET)?;
        Ok((lua, deadline))
    }

    fn run_updates(
        &mut self,
        ctx: &egui::Context,
        now: Instant,
        ui_visible: bool,
        ui_rect: &UiRectHandle,
    ) {
        let elapsed = now.duration_since(self.last_tick);
        self.last_tick = now;

        // OnUpdate runs once per rendered frame. The dt is the real frame
        // interval; a slow callback simply stretches it and lowers the
        // achieved frequency. Budget overruns are advisory warnings only.
        let result = self.lua.scope(|scope| {
            let module = create_ui_module(&self.lua, scope, ctx, ui_visible, ui_rect)?;
            self.lua.globals().set("ui", module)?;
            let dt = if self.control.paused.load(Ordering::Relaxed) {
                0.0
            } else {
                elapsed.min(MAX_FRAME_DELTA).as_secs_f32()
            };
            match execution::call_unbudgeted(&self.lua, "OnUpdate", dt) {
                Ok(callback_elapsed) => {
                    self.consecutive_healthy_updates =
                        self.consecutive_healthy_updates.saturating_add(1);
                    if callback_elapsed > UPDATE_BUDGET {
                        tracing::warn!(
                            script = %self.script_path.display(),
                            elapsed_us = callback_elapsed.as_micros() as u64,
                            budget_us = UPDATE_BUDGET.as_micros() as u64,
                            "Lua OnUpdate exceeded advisory budget"
                        );
                    }
                    if self.consecutive_healthy_updates >= UPDATE_HEALTHY_FRAMES_TO_CLEAR_ERROR
                        && self
                            .last_error
                            .as_deref()
                            .is_some_and(|error| error.starts_with("OnUpdate failed:"))
                    {
                        self.last_error = None;
                    }
                }
                Err(error) => {
                    self.consecutive_healthy_updates = 0;
                    self.last_error = Some(format!("OnUpdate failed: {error}"));
                }
            }
            Ok::<(), mlua::Error>(())
        });
        if let Err(error) = result {
            self.last_error = Some(format!("OnUpdate failed: {error}"));
        }

        if let Some(error) = self.last_error.clone() {
            let mut reload = false;
            let response = egui::Window::new("Lua error").show(ctx, |ui| {
                ui.label(error);
                if ui.button("Reload script").clicked() {
                    reload = true;
                }
            });
            if let Some(response) = response {
                ui_api::record_ui_rect(ui_rect, response.response.rect);
            }
            if reload {
                self.reload();
            }
        }
    }

    fn check_hot_reload(&mut self, now: Instant) {
        if now.duration_since(self.last_reload_poll) < RELOAD_POLL_INTERVAL {
            return;
        }
        self.last_reload_poll = now;
        let modified = script_modified(&self.script_path);
        if modified.is_some() && modified != self.script_modified {
            self.script_modified = modified;
            self.reload();
        }
    }

    fn reload(&mut self) {
        match Self::build_vm(
            &self.script_path,
            Arc::clone(&self.control),
            Arc::clone(&self.draw_commands),
            Arc::clone(&self.config),
            Arc::clone(&self.keyboard),
        ) {
            Ok((new_lua, new_deadline)) => {
                if let Err(error) = execution::call_budgeted(
                    &self.lua,
                    &self.deadline,
                    "OnDestroy",
                    (),
                    DESTROY_BUDGET,
                ) {
                    eprintln!("OnDestroy failed during reload: {error}");
                }
                self.lua = new_lua;
                self.deadline = new_deadline;
                self.last_error = None;
                self.consecutive_healthy_updates = 0;
                self.last_tick = Instant::now();
                self.script_modified = script_modified(&self.script_path);
            }
            Err(error) => self.last_error = Some(format!("Reload failed: {error}")),
        }
    }
}

impl Drop for LuaRuntime {
    fn drop(&mut self) {
        if let Err(error) =
            execution::call_budgeted(&self.lua, &self.deadline, "OnDestroy", (), DESTROY_BUDGET)
        {
            eprintln!("OnDestroy failed during shutdown: {error}");
        }
    }
}

fn script_modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

// Coroutine suspension happens entirely on the Lua thread. The IPC worker
// only produces task results, so no Lua object crosses a thread boundary.

#[cfg(test)]
mod tests {
    use super::*;

    struct ScriptFixture(PathBuf);

    impl ScriptFixture {
        fn new(files: &[(&str, &str)]) -> Self {
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "ks-current-startup-test-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(root.join("scripts")).unwrap();
            for (name, source) in files {
                fs::write(root.join("scripts").join(name), source).unwrap();
            }
            Self(root)
        }

        fn scripts(&self) -> PathBuf {
            self.0.join("scripts")
        }
    }

    impl Drop for ScriptFixture {
        fn drop(&mut self) {
            // Delete only files in this test's unique directory; never recurse.
            if let Ok(files) = fs::read_dir(self.scripts()) {
                for file in files.flatten() {
                    let _ = fs::remove_file(file.path());
                }
            }
            let _ = fs::remove_dir(self.scripts());
            let _ = fs::remove_file(self.0.join("config.json"));
            let _ = fs::remove_dir(&self.0);
        }
    }

    #[test]
    fn deferred_startup_isolates_failures_and_error_window_accepts_input() {
        let fixture = ScriptFixture::new(&[
            ("a_syntax.lua", "function broken("),
            ("b_start.lua", "function OnStart() error('bad startup') end"),
            ("c_good.lua", "function OnStart() started = true end"),
            ("_disabled.lua", "error('must not load')"),
        ]);
        let mut manager = LuaRuntimeManager::new(fixture.scripts()).unwrap();
        assert!(manager.runtimes.is_empty());
        assert_eq!(manager.pending_scripts.len(), 3);
        let first_frame = Instant::now();
        manager.load_pending_script(first_frame);
        let scheduled = manager.script_load_after.unwrap();
        assert_eq!(scheduled, first_frame + SCRIPT_START_DELAY);
        manager.load_pending_script(scheduled - Duration::from_millis(1));
        assert_eq!(manager.pending_scripts.len(), 3);
        let ready = scheduled + Duration::from_millis(1);
        manager.load_pending_script(ready);
        assert_eq!(manager.pending_scripts.len(), 2);
        assert_eq!(manager.startup_errors.len(), 1);
        manager.load_pending_script(ready);
        manager.load_pending_script(ready);
        assert_eq!(manager.startup_errors.len(), 2);
        assert_eq!(manager.runtimes.len(), 1);
        assert!(manager.runtimes[0]
            .lua
            .globals()
            .get::<bool>("started")
            .unwrap());
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            manager.frame(ui.ctx(), ready, true)
        });
        output.textures_delta.clear(); // Headless test: there is no GPU renderer.
        assert!(
            manager.ui_rect.get().is_some(),
            "startup error window must register an interactive region"
        );
        for (path, _) in &manager.startup_errors {
            fs::write(path, "function OnStart() repaired = true end").unwrap();
        }
        manager.retry_failed_scripts();
        assert!(manager.startup_errors.is_empty());
        manager.load_pending_script(ready);
        manager.load_pending_script(ready);
        assert_eq!(manager.runtimes.len(), 3);
        for runtime in &manager.runtimes[1..] {
            assert!(runtime.lua.globals().get::<bool>("repaired").unwrap());
        }
    }

    #[test]
    fn top_level_and_onstart_runaways_remain_bounded() {
        let fixture = ScriptFixture::new(&[
            ("a_top_level.lua", "while true do end"),
            ("b_onstart.lua", "function OnStart() while true do end end"),
            ("c_good.lua", "function OnStart() started = true end"),
        ]);
        let mut manager = LuaRuntimeManager::new(fixture.scripts()).unwrap();
        manager.load_pending_script(Instant::now());
        let ready = manager.script_load_after.unwrap() + Duration::from_millis(1);
        for _ in 0..3 {
            manager.load_pending_script(ready);
        }
        assert_eq!(manager.startup_errors.len(), 2);
        for (_, error) in &manager.startup_errors {
            assert!(error.contains("execution budget exceeded"));
        }
        assert_eq!(manager.runtimes.len(), 1);
        assert!(manager.runtimes[0]
            .lua
            .globals()
            .get::<bool>("started")
            .unwrap());
    }

    #[test]
    fn parses_decimal_and_hex_addresses_without_float_conversion() {
        assert_eq!(
            engine_api::parse_address("140702365450240").unwrap().get(),
            140702365450240
        );
        assert_eq!(
            engine_api::parse_address("0x7FF812345000").unwrap().get(),
            0x7FF8_1234_5000
        );
        assert_eq!(engine_api::parse_address("0").unwrap().get(), 0);
        assert!(engine_api::parse_address("not-an-address").is_err());
    }

    #[test]
    fn execution_hook_stops_runaway_script() {
        let lua = Lua::new();
        let deadline = execution::install_hook(&lua);
        lua.load("function OnUpdate() while true do end end")
            .exec()
            .unwrap();
        let error = execution::call_budgeted(
            &lua,
            &deadline,
            "OnUpdate",
            0.033_f32,
            Duration::from_millis(1),
        )
        .unwrap_err();
        assert!(error.to_string().contains("execution budget exceeded"));
    }
}
