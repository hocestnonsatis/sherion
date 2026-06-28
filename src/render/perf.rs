use std::time::Duration;

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderTimings {
    pub scene: Duration,
    pub gpu: Duration,
}

#[derive(Clone, Copy, Debug)]
pub struct PerfStatsSnapshot {
    pub frame_ms: f32,
    pub capture_ms: f32,
    pub scene_ms: f32,
    pub gpu_ms: f32,
    pub render_ms: f32,
    pub fps: f32,
    pub panes: usize,
    pub full_panes: usize,
    pub dirty_rows: usize,
    pub skipped_frames: u64,
}

impl Default for PerfStatsSnapshot {
    fn default() -> Self {
        Self {
            frame_ms: 0.0,
            capture_ms: 0.0,
            scene_ms: 0.0,
            gpu_ms: 0.0,
            render_ms: 0.0,
            fps: 0.0,
            panes: 0,
            full_panes: 0,
            dirty_rows: 0,
            skipped_frames: 0,
        }
    }
}

impl PerfStatsSnapshot {
    pub fn record(
        &mut self,
        capture: Duration,
        scene: Duration,
        gpu: Duration,
        frame: Duration,
        panes: usize,
        full_panes: usize,
        dirty_rows: usize,
        skipped: bool,
    ) {
        if skipped {
            self.skipped_frames = self.skipped_frames.saturating_add(1);
            return;
        }
        self.frame_ms = duration_ms(frame);
        self.capture_ms = duration_ms(capture);
        self.scene_ms = duration_ms(scene);
        self.gpu_ms = duration_ms(gpu);
        self.render_ms = self.scene_ms + self.gpu_ms;
        self.fps = if self.frame_ms > 0.0 {
            1000.0 / self.frame_ms
        } else {
            0.0
        };
        self.panes = panes;
        self.full_panes = full_panes;
        self.dirty_rows = dirty_rows;
    }
}

fn duration_ms(duration: Duration) -> f32 {
    duration.as_secs_f32() * 1000.0
}
