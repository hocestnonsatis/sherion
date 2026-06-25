use std::num::NonZeroUsize;
use std::sync::Arc;

use anyhow::{Context, Result};
use pollster::block_on;
use vello::util::{RenderContext, RenderSurface};
use vello::{AaConfig, AaSupport, Renderer as VelloRenderer, RendererOptions, RenderParams, Scene};
use vello::kurbo::{Affine, Rect};
use vello::peniko::color::AlphaColor;
use vello::peniko::{Brush, Fill};
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::config::Config;
use crate::render::chrome::ChromeMetrics;
use crate::render::menu::{MenuEntry, MenuLayout, MenuRenderer};
use crate::render::scene::SceneBuilder;
use crate::render::tab_bar::TabBarEntry;
use crate::render::tab_strip::{TabStripLayout, TabStripRenderer};
use crate::render::title_bar::{TitleBarLayout, TitleBarRenderer};
use crate::render::theme::UiTheme;
use crate::render::TerminalLayout;

pub struct ChromeFrame<'a> {
    pub metrics: ChromeMetrics,
    pub tab_entries: &'a [TabBarEntry],
    pub window_title: &'a str,
    pub menu_open: bool,
    pub menu_entries: &'a [MenuEntry],
    pub tab_scroll_offset: f64,
    pub window_height: f64,
    pub sidebar_collapsed: bool,
    pub bell_flash: bool,
    pub terminal_opacity: f32,
}

pub struct GpuRenderer {
    render_context: RenderContext,
    surface: Option<RenderSurface<'static>>,
    vello: Option<VelloRenderer>,
    scene_builder: SceneBuilder,
    tab_strip_renderer: TabStripRenderer,
    title_bar_renderer: TitleBarRenderer,
    menu_renderer: MenuRenderer,
    font_cx: parley::FontContext,
    layout: TerminalLayout,
    tab_strip_layout: Option<TabStripLayout>,
    title_bar_layout: Option<TitleBarLayout>,
    menu_layout: Option<MenuLayout>,
    config: Config,
}

impl GpuRenderer {
    pub fn new(window: Arc<Window>, layout: TerminalLayout, config: Config) -> Result<Self> {
        let size = window.inner_size();
        let mut render_context = RenderContext::new();

        let mut surface = block_on(render_context.create_surface(
            Arc::clone(&window),
            size.width.max(1),
            size.height.max(1),
            wgpu::PresentMode::AutoVsync,
        ))
        .context("failed to create wgpu surface")?;

        // Enable real window transparency. Vello's helper defaults to
        // `CompositeAlphaMode::Auto`, which resolves to `Opaque` on most
        // compositors and discards the alpha channel entirely. Pick an alpha
        // mode that actually blends the window with whatever is behind it.
        if let Some(alpha_mode) =
            block_on(preferred_alpha_mode(&render_context, &surface))
        {
            if surface.config.alpha_mode != alpha_mode {
                surface.config.alpha_mode = alpha_mode;
                render_context.configure_surface(&surface);
            }
        }

        let device = &render_context.devices[surface.dev_id].device;
        let mut options = RendererOptions::default();
        options.use_cpu = false;
        options.antialiasing_support = AaSupport::all();
        options.num_init_threads = NonZeroUsize::new(1);

        let vello = VelloRenderer::new(device, options)
            .context("failed to create vello renderer")?;

        Ok(Self {
            render_context,
            surface: Some(surface),
            vello: Some(vello),
            scene_builder: SceneBuilder::new(),
            tab_strip_renderer: TabStripRenderer,
            title_bar_renderer: TitleBarRenderer,
            menu_renderer: MenuRenderer,
            font_cx: parley::FontContext::new(),
            layout,
            tab_strip_layout: None,
            title_bar_layout: None,
            menu_layout: None,
            config,
        })
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        if size.width > 0 && size.height > 0 {
            self.render_context
                .resize_surface(surface, size.width, size.height);
        }
    }

    pub fn set_layout(&mut self, layout: TerminalLayout) {
        self.layout = layout;
    }

    pub fn set_config(&mut self, config: Config) {
        self.config = config;
    }

    pub fn present_clear(&mut self) -> Result<()> {
        let width = self
            .surface
            .as_ref()
            .map(|surface| surface.config.width)
            .unwrap_or(1);
        let height = self
            .surface
            .as_ref()
            .map(|surface| surface.config.height)
            .unwrap_or(1);

        let mut scene = Scene::new();
        let theme = UiTheme::from_config(&self.config);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &theme.chrome_brush(),
            None,
            &Rect::new(0.0, 0.0, width as f64, height as f64),
        );
        self.render_scene(&scene, 1.0)
    }

    pub fn tab_strip_layout(&self) -> Option<&TabStripLayout> {
        self.tab_strip_layout.as_ref()
    }

    pub fn title_bar_layout(&self) -> Option<&TitleBarLayout> {
        self.title_bar_layout.as_ref()
    }

    pub fn menu_layout(&self) -> Option<&MenuLayout> {
        self.menu_layout.as_ref()
    }

    pub fn render(
        &mut self,
        content: alacritty_terminal::term::RenderableContent<'_>,
        frame: ChromeFrame<'_>,
    ) -> Result<()> {
        let width = self
            .surface
            .as_ref()
            .map(|surface| surface.config.width)
            .unwrap_or(1) as f64;

        let mut scene = Scene::new();

        self.scene_builder.build(
            &mut scene,
            content,
            self.layout,
            &self.config,
            &mut self.font_cx,
            frame.terminal_opacity,
        );

        if frame.bell_flash {
            let x_off = f64::from(self.layout.content_offset_x);
            let y_off = f64::from(self.layout.content_offset_y);
            let w = f64::from(self.layout.cell_width) * f64::from(self.layout.cols);
            let h = f64::from(self.layout.cell_height) * f64::from(self.layout.rows);
            let flash = Brush::Solid(AlphaColor::from_rgba8(255, 255, 180, 64));
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                &flash,
                None,
                &Rect::new(x_off, y_off, x_off + w, y_off + h),
            );
        }

        let title_height = f64::from(frame.metrics.title_bar.height);
        let sidebar_height = (frame.window_height - title_height).max(0.0);
        let tab_layout = TabStripLayout::compute(
            frame.metrics.tab_strip,
            title_height,
            sidebar_height,
            frame.tab_entries,
            frame.sidebar_collapsed,
        );
        self.tab_strip_renderer.render(
            &mut scene,
            &tab_layout,
            frame.tab_entries,
            frame.tab_scroll_offset,
            &self.config,
            &mut self.font_cx,
            &mut self.scene_builder.layout_cx,
            self.layout.font_size,
        );
        self.tab_strip_layout = Some(tab_layout);

        // Title bar spans the full window width, sitting above both the sidebar and terminal.
        let title_layout = TitleBarLayout::compute(frame.metrics.title_bar, 0.0, width);
        let menu_anchor = if frame.menu_open {
            Some(title_layout.hamburger_anchor())
        } else {
            None
        };
        self.title_bar_renderer.render(
            &mut scene,
            &title_layout,
            frame.window_title,
            &self.config,
            &mut self.font_cx,
            &mut self.scene_builder.layout_cx,
            self.layout.font_size,
            frame.menu_open,
        );
        self.title_bar_layout = Some(title_layout);

        if let Some((anchor_x, anchor_y)) = menu_anchor {
            let menu_layout =
                MenuLayout::compute(anchor_x, anchor_y, width, frame.menu_entries);
            self.menu_renderer.render(
                &mut scene,
                &menu_layout,
                frame.menu_entries,
                &self.config,
                &mut self.font_cx,
                &mut self.scene_builder.layout_cx,
                self.layout.font_size,
            );
            self.menu_layout = Some(menu_layout);
        } else {
            self.menu_layout = None;
        }

        self.render_scene(&scene, frame.terminal_opacity)
    }

    fn render_scene(&mut self, scene: &Scene, terminal_opacity: f32) -> Result<()> {
        let surface = self
            .surface
            .as_mut()
            .context("renderer surface not initialized")?;
        let vello = self
            .vello
            .as_mut()
            .context("vello renderer not initialized")?;

        let width = surface.config.width;
        let height = surface.config.height;
        if width == 0 || height == 0 {
            return Ok(());
        }

        let device = &self.render_context.devices[surface.dev_id].device;
        let queue = &self.render_context.devices[surface.dev_id].queue;

        let theme = UiTheme::from_config(&self.config);
        let base_color = if terminal_opacity < 1.0 - f32::EPSILON {
            AlphaColor::from_rgba8(0, 0, 0, 0)
        } else {
            theme.chrome
        };

        vello
            .render_to_texture(
                device,
                queue,
                scene,
                &surface.target_view,
                &RenderParams {
                    base_color,
                    width,
                    height,
                    antialiasing_method: AaConfig::Msaa16,
                },
            )
            .context("failed to render vello scene")?;

        let surface_texture = match surface.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Outdated => return Ok(()),
            other => {
                return Err(anyhow::anyhow!(
                    "failed to acquire surface texture: {other:?}"
                ));
            }
        };
        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("sherion-blit"),
        });

        surface.blitter.copy(
            device,
            &mut encoder,
            &surface.target_view,
            &surface_view,
        );

        queue.submit(Some(encoder.finish()));
        surface_texture.present();
        self.render_context.configure_surface(surface);

        Ok(())
    }
}

/// Picks a non-opaque surface alpha mode so the compositor blends the window
/// with the desktop behind it. Returns `None` if only opaque modes exist.
async fn preferred_alpha_mode(
    render_context: &RenderContext,
    surface: &RenderSurface<'_>,
) -> Option<wgpu::CompositeAlphaMode> {
    let adapter = render_context
        .instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::None,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface.surface),
        })
        .await
        .ok()?;

    let caps = surface.surface.get_capabilities(&adapter);
    // Vello writes premultiplied-alpha pixels into its target texture, so
    // PreMultiplied is the natural match. Fall back to the other blending
    // modes if the compositor doesn't offer it.
    [
        wgpu::CompositeAlphaMode::PreMultiplied,
        wgpu::CompositeAlphaMode::PostMultiplied,
        wgpu::CompositeAlphaMode::Inherit,
    ]
    .into_iter()
    .find(|mode| caps.alpha_modes.contains(mode))
}

pub type Renderer = GpuRenderer;
