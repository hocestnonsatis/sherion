# Sherion — Terminal Özellik Envanteri

Bu dosya bir terminal emülatöründe olması beklenen özellikleri listeler ve Sherion'daki durumlarını işaretler.

**Durum işaretleri**

- `[x]` — Var (tam uygulanmış)
- `[~]` — Kısmen var (sınırlı veya eksik)
- `[ ]` — Yok

Son güncelleme: 2026-06-29

> **Durum:** Özellik envanterinde `[ ]` (uygulanmamış) madde kalmadı. Kalan işler sağlama alma ve mimari sertleştirmedir (aşağıdaki `[~]` maddeler test/CI ile kapatıldı veya platform sınırı olarak belgelendi).

---

## Özet — Kritik eksikler

- [x] Mouse reporting (SGR / 1000 / 1002 / 1003) — vim, tmux, htop mouse alabilir
- [x] Bracketed paste — çok satırlı yapıştırma güvenli
- [x] IME / dead key — winit Ime + KeyEvent.text desteği
- [x] DECCKM (application cursor keys) — SS3 ok tuşları
- [x] Yapılandırılabilir keybinding'ler
- [x] Strikethrough / dim text attribute'ları
- [x] Gerçek split pane (tek tab → iki PTY)
- [x] Tüm scrollback'te arama + klavye ile scroll
- [x] Sesli bell
- [x] Fullscreen toggle
- [x] OSC 8 explicit hyperlink

---

## 1. PTY / Süreç Yönetimi

| Özellik | Durum | Not |
|---------|:-----:|-----|
| Shell başlatma (ayrı thread) | [x] | `alacritty_terminal` event loop |
| Çalışma dizini (CWD) takibi | [x] | OSC 7 + Linux `/proc/{pid}/cwd` |
| Özel shell + env (`TERM`, `COLORTERM`) | [x] | `xterm-256color` / `truecolor` |
| Süreç çıkışında tab kapatma | [x] | `TerminalEvent::Exit` |
| Busy / foreground süreç algılama | [x] | Unix `tcgetpgrp`; Windows/macOS recent-output heuristic (`busy_heuristic_ms`) |
| Windows / ConPTY desteği | [x] | portable-pty ConPTY; output tap all platforms |
| Shell argümanları (config'den) | [x] | `terminal.shell_args` |
| OSC 7 CWD raporlama | [x] | PTY tap + `reported_cwd` API |

### Detay checklist

- [x] PTY okuma/yazma UI thread'inden ayrı
- [x] `PtySession::spawn` / `spawn_with_working_directory`
- [x] Yeni tab aktif tab CWD'sini miras alır
- [x] `is_busy()` — Unix `tcgetpgrp`
- [x] `is_busy()` — non-Unix recent-output heuristic (`busy_heuristic_ms`, `src/pty/busy.rs`)
- [x] `current_working_directory()` — OSC 7 + Linux `/proc`
- [x] ConPTY / Windows PTY
- [x] Config'den shell argümanları

---

## 2. VT / ANSI Ayrıştırma

| Özellik | Durum | Not |
|---------|:-----:|-----|
| VT parser | [x] | `alacritty_terminal` (vte) |
| Truecolor / 256 / 16 renk | [x] | `color_to_brush` |
| Bold / italic / underline / reverse | [x] | `GlyphStyle::from_cell` |
| Wide char (CJK) | [x] | `Flags::WIDE_CHAR` |
| Cursor şekilleri | [x] | block / beam / underline / hollow |
| Strikethrough | [x] | `Flags::STRIKEOUT` çizimi |
| Dim / faint | [x] | Alpha solması |
| Çift / curly / renkli underline | [x] | double/curly/dotted/dashed |
| Blink | [x] | Cursor blink + SGR 5 metin blink (vendor `Flags::BLINK`) |
| Zerowidth combining char shaping | [x] | `push_cell_text` + atlas kısa run; regression testler |

### Detay checklist

- [x] 16 ANSI renk
- [x] 256 renk paleti
- [x] Truecolor (24-bit)
- [x] Bold
- [x] Italic
- [x] Underline
- [x] Inverse / reverse video
- [x] Wide character (2 sütun)
- [x] Cursor: Block
- [x] Cursor: Underline
- [x] Cursor: Beam
- [x] Cursor: Hollow block
- [x] Cursor: Hidden
- [x] Pencere başlığı (OSC title)
- [x] Strikethrough
- [x] Dim / faint (alpha veya renk solması)
- [x] Cursor blink (DECSCUSR / mode 12)
- [x] Metin blink (SGR 5 — vendor `alacritty_terminal` patch + render timer)
- [x] DECSCUSR genişletilmiş cursor stilleri (config + parser shape seti)

---

## 3. Render

| Özellik | Durum | Not |
|---------|:-----:|-----|
| GPU backend | [x] | vello + wgpu |
| Font shaping + fallback + emoji | [x] | parley, Nerd Font + Noto Emoji |
| Dirty-row takibi | [x] | `FrameDamage` |
| VSync | [x] | `AutoVsync` |
| Opacity / şeffaflık | [x] | Menüden ayarlanır |
| Tema (Light / Dark / Auto) | [x] | `ThemeMode` |
| Glyph cache | [x] | Shaped-run cache + swash atlas (fallback font zinciri) |
| Ligatures | [x] | `[font] ligatures`, style-run shaping + clip |
| Rasterize glyph atlas | [x] | `[font] glyph_atlas`, swash + vello ImageBrush |
| Scissor / partial GPU upload | [x] | Partial damage row-band clip layers |
| Arka plan görseli / shader | [x] | `[appearance].background_image` + `background_shader` preset |

### Detay checklist

- [x] vello + wgpu render pipeline
- [x] parley ile text shaping
- [x] Font fallback listesi
- [x] Emoji desteği (Noto Color Emoji)
- [x] `FrameDamage::Full` / `Partial` / `None`
- [x] Hasarlı satır bazlı capture
- [x] Per-pane persistent scene
- [x] Chrome scene cache (sidebar, title bar)
- [x] Glyph shaped-run cache (`GlyphCache`)
- [x] VSync (`PresentMode::AutoVsync`)
- [x] Terminal opacity
- [x] Light / Dark / Auto tema
- [x] Özelleştirilebilir fg / bg / cursor rengi
- [x] Font zoom (Ctrl+wheel, menü)
- [x] Ligatures (fi, --> vb.) — config toggle
- [x] GPU rasterize glyph atlas (swash)
- [x] Scissor ile kısmi GPU çizim (partial row clips)
- [x] Özel arka plan görseli (`cover` / `contain` / `tile` / `center`)
- [x] Özel arka plan shader (`vignette` / `scanlines` / `noise`)

---

## 4. Scrollback

| Özellik | Durum | Not |
|---------|:-----:|-----|
| Scrollback buffer | [x] | Varsayılan 10k satır, config'den ayarlanır |
| Mouse wheel scroll | [x] | `scroll_display` |
| Clear scrollback | [x] | Ctrl+Shift+K |
| Scrollbar UI | [x] | Sağ kenar track + thumb |
| Klavye scroll (Shift+PageUp/Down) | [x] | Scrollback kaydırma |
| Scroll-on-output | [x] | `[ui].follow_output` toggle |
| Jump-to-prompt | [x] | Alt+Shift+Up/Down |

### Detay checklist

- [x] `scrollback_lines` config
- [x] Mouse wheel (line + pixel delta)
- [x] Scroll to bottom
- [x] Clear history (`clear_scrollback`)
- [x] Görsel scrollbar
- [x] Prompt'a atlama

---

## 5. Seçim & Pano

| Özellik | Durum | Not |
|---------|:-----:|-----|
| Mouse seçim | [x] | Simple / semantic / line |
| Kopyala / yapıştır | [x] | `arboard` |
| Sağ tık yapıştır | [x] | — |
| Otomatik kopyala (seçim sonrası) | [x] | Mouse release |
| Bracketed paste | [x] | `\e[200~` … `\e[201~` sarmalama |
| Primary selection (orta tık) | [x] | Linux primary + fallback |
| Blok / dikdörtgen seçim | [x] | Alt + sürükle |
| OSC 52 clipboard | [x] | `[terminal] osc52` + event handler |
| Yapıştırma sanitizasyonu | [x] | `[terminal].sanitize_paste` |

### Detay checklist

- [x] Tek tık seçim
- [x] Çift tık kelime seçimi
- [x] Üç tık satır seçimi
- [x] Sürükleyerek seçim güncelleme
- [x] Seçim highlight (inverse renk)
- [x] Ctrl+Shift+C / menü ile kopyala
- [x] Ctrl+Shift+V / Shift+Insert ile yapıştır
- [x] Sağ tık yapıştır
- [x] Bracketed paste (`\e[200~` … `\e[201~`)
- [x] X11 primary selection
- [x] Rectangular (block) selection
- [x] OSC 52 programmatic clipboard (copy/paste)
- [x] Yapıştırmada newline / kontrol karakteri filtreleme

---

## 6. Sekmeler (Tabs)

| Özellik | Durum | Not |
|---------|:-----:|-----|
| Aç / kapat / geçiş / numara | [x] | Ctrl+Shift+T/W, Ctrl+Tab, Ctrl+1..9 |
| Rename | [x] | F2, menü, palet |
| Yeni pencereye ayır (detach) | [x] | Ctrl+Shift+N |
| Sidebar (resizable / collapse / scroll) | [x] | `TabStripRenderer` |
| Busy göstergesi + süre | [x] | — |
| CWD alt başlık | [x] | `short_path` |
| Duplicate (geçmiş kopyası) | [x] | Başlık + CWD + scrollback metin klonu |
| Sürükle-bırak sıralama | [x] | Sidebar tab strip |
| Tab renk / pin | [x] | Menü + komut paleti |

### Detay checklist

- [x] Yeni tab (Ctrl+Shift+T)
- [x] Tab kapat (Ctrl+Shift+W)
- [x] Sonraki / önceki tab (Ctrl+Tab)
- [x] Tab numarası ile geçiş (Ctrl+1..9)
- [x] Tab rename overlay (F2)
- [x] Tab detach → yeni pencere
- [x] Dikey sidebar tab strip
- [x] Sidebar genişlik ayarı (sürükle)
- [x] Sidebar collapse (ikon modu)
- [x] Tab strip scroll
- [x] Tab başına close (×) ve detach (↗)
- [x] Busy indicator + elapsed süre
- [x] CWD subtitle
- [x] Duplicate tab (başlık suffix, CWD miras, scrollback klonu)
- [x] Tab sürükle-bırak sıralama
- [x] Tab pin / sabitleme
- [x] Tab renklendirme

---

## 7. Split / Pane

| Özellik | Durum | Not |
|---------|:-----:|-----|
| Grid görünüm (tab'ları döşer) | [x] | Max 9, otomatik √n grid |
| Gerçek split (tek tab → 2 PTY) | [x] | `SplitNode` ağacı |
| Mouse ile split boyutlandırma | [x] | Divider sürükleme |
| Klavye ile pane gezinme | [x] | Ctrl+Shift+ok |
| Nested / h-v split ağacı | [x] | `SplitNode` ağacı |

### Detay checklist

- [x] Single view modu
- [x] Grid view modu (menü / palet)
- [x] Otomatik grid layout (max 9 pane)
- [x] Pane tıklayınca focus
- [x] Focused pane accent border
- [x] Per-pane terminal resize
- [x] Per-pane frame capture
- [x] Bağımsız split (aynı tab içinde 2 shell)
- [x] Split boyutu mouse ile ayarlama
- [x] Klavye pane navigation (Ctrl+Shift+arrow)
- [x] Yatay / dikey split ağacı

---

## 8. Arama

| Özellik | Durum | Not |
|---------|:-----:|-----|
| Arama overlay | [x] | Ctrl+Shift+F |
| Eşleşme highlight | [x] | Sarı / turuncu aktif eşleşme |
| Navigasyon (Enter / Shift+Enter) | [x] | Sonraki / önceki |
| Kapatma (Esc, ×, dış tık) | [x] | — |
| Tüm scrollback'te arama | [x] | Grid taraması |
| Regex | [x] | Alt+R toggle |
| Büyük-küçük harf duyarlılığı | [x] | Alt+C / Ctrl+Shift+C toggle |
| Tam kelime eşleşmesi | [x] | Alt+W toggle |
| Eşleşmeye otomatik scroll | [x] | Enter / Shift+Enter |

### Detay checklist

- [x] Search overlay UI
- [x] Case-insensitive substring arama
- [x] Match count gösterimi
- [x] Aktif eşleşme vurgusu
- [x] Enter → sonraki eşleşme
- [x] Shift+Enter → önceki eşleşme
- [x] Esc ile kapat
- [x] Scrollback boyunca arama
- [x] Off-screen eşleşmeye git
- [x] Regex desteği
- [x] Case-sensitive toggle
- [x] Whole word toggle

---

## 9. Komut Paleti

| Özellik | Durum | Not |
|---------|:-----:|-----|
| Palet (Ctrl+Shift+P) | [x] | Komutlar + tab atlama |
| Filtreleme | [x] | Fuzzy subsequence scoring |
| Ok tuşuyla seçim | [x] | ↑↓ ile seçim |
| Keybinding ipuçları | [x] | Komut satırında gösterilir |

### Detay checklist

- [x] Komut paleti toggle (Ctrl+Shift+P)
- [x] Tab listesi (hızlı geçiş)
- [x] Sabit komut listesi (New Tab, Copy, Paste, Theme, Zoom, Quit vb.)
- [x] Substring filtreleme
- [x] Enter ile çalıştır
- [x] Fuzzy scoring / ranking
- [x] Komut yanında kısayol gösterimi

---

## 10. Linkler / Hyperlink

| Özellik | Durum | Not |
|---------|:-----:|-----|
| URL algılama (http/https/file) | [x] | Heuristic satır taraması |
| Ctrl+hover highlight | [x] | Mavi + underline |
| Ctrl+click ile aç | [x] | `xdg-open` (Linux) |
| OSC 8 explicit hyperlink | [x] | `cell.hyperlink()` |
| macOS / Windows opener | [x] | `open` / `cmd start` |
| Çok satırlı URL | [x] | `src/links.rs` row-window scanner + span map |
| www. / mailto: algılama | [x] | `www.` → https normalize |

### Detay checklist

- [x] `http://` algılama
- [x] `https://` algılama
- [x] `file://` algılama
- [x] Hover highlight
- [x] Pointer cursor (Ctrl basılıyken)
- [x] `xdg-open` ile açma (stdio null)
- [x] OSC 8 (`\e]8;;url\a` … `\e]8;;\a`)
- [x] `open` (macOS) / `start` (Windows)
- [x] `www.` prefix
- [x] `mailto:` algılama
- [x] Satırlar arası kırılmış URL birleştirme (WRAPLINE + span highlight)

---

## 11. Girdi (Klavye / IME)

| Özellik | Durum | Not |
|---------|:-----:|-----|
| Klavye → byte eşleme | [x] | `key_event_to_bytes` |
| Modifier (Ctrl / Alt / Shift) | [x] | — |
| Uygulama kısayolları | [x] | Tab, zoom, copy/paste vb. |
| IME / dead key / kompozisyon | [x] | `Ime` event + `KeyEvent.text` |
| DECCKM (application cursor keys) | [x] | SS3 (`\eOA` vb.) |
| Keypad application mode | [x] | `APP_KEYPAD` + fiziksel numpad |
| Kitty keyboard protocol | [x] | `CSI > ... u` mode parser + `CSI u` encoder |
| modifyOtherKeys | [x] | `CSI u` encoding + PTY tap state |
| Yapılandırılabilir kısayollar | [x] | `[keybindings]` TOML |

### Detay checklist

- [x] Enter, Backspace, Tab, Esc
- [x] Arrow keys
- [x] Home, End, Delete, PageUp, PageDown
- [x] Ctrl+letter control codes
- [x] Alt+char (ESC prefix)
- [x] Ctrl+Space (NUL)
- [x] App-level shortcut ayrımı
- [x] `Ime` event handling
- [x] Dead key / compose (`Key::Dead` + `event.text`)
- [x] DECCKM — SS3 cursor keys (`\eOA` vb.)
- [x] Keypad application / numeric mode
- [x] Kitty keyboard protocol
- [x] modifyOtherKeys
- [x] Config'den keybinding override

---

## 12. Mouse Protokolü (uygulamaya raporlama)

| Özellik | Durum | Not |
|---------|:-----:|-----|
| SGR mouse (1006) | [x] | `src/mouse.rs` |
| Normal tracking (1000) | [x] | `MOUSE_REPORT_CLICK` |
| Button-event tracking (1002) | [x] | `MOUSE_DRAG` |
| Any-event tracking (1003) | [x] | `MOUSE_MOTION` |
| X10 mouse | [x] | Legacy encoding (SGR yokken) |
| Focus reporting (1004) | [x] | `\e[I` / `\e[O` |

### Detay checklist

- [x] Mouse mode escape sequence'lerine uyum
- [x] `term.mode()` ile mouse tracking okuma
- [x] Tıklama → PTY'ye SGR raporlama
- [x] Sürükleme → motion raporlama
- [x] Scroll wheel → uygulamaya raporlama (mode açıkken)
- [x] Focus in/out raporlama

> **Not:** Shift basılıyken mouse mode bypass edilir (metin seçimi için).

---

## 13. Config / Kalıcılık

| Özellik | Durum | Not |
|---------|:-----:|-----|
| TOML config (`sherion.toml`) | [x] | Font, renk, terminal, bell, ui, session |
| UI tercihleri kalıcılığı | [x] | Opacity, zoom, sidebar, view mode |
| Session restore (tab CWD) | [x] | Max 16 tab |
| `SHERION_CONFIG` env override | [x] | — |
| Keybinding config | [x] | `[keybindings]` bölümü |
| Tam 16/256 palet override | [x] | OSC 4/10/11/12 via config |
| Pencere konum/boyut kalıcılığı | [x] | `[window]` width/height/x/y |
| Live config reload | [x] | `notify` ile `sherion.toml` watch |
| Cursor style config | [x] | `[terminal].cursor_style` |
| Profil / çoklu config | [x] | `[profiles.*]`, `--profile`, palette switch |

### Detay checklist

- [x] `[font]` family / size / fallback
- [x] `[colors]` foreground / background / cursor
- [x] `[terminal]` scrollback_lines / shell
- [x] `[bell]` visual
- [x] `[appearance]` theme / opacity
- [x] `[ui]` font_zoom / sidebar_width / sidebar_collapsed / view_mode
- [x] `[session]` restore_tabs / cwd
- [x] `Config::save()` ile diske yazma
- [x] Kapanışta tercih kaydetme
- [x] Keybinding bölümü
- [x] 16 / 256 renk paleti override
- [x] Pencere x/y/width/height
- [x] Config dosyası watch / hot reload

---

## 14. Pencere

| Özellik | Durum | Not |
|---------|:-----:|-----|
| Çoklu pencere | [x] | `HashMap<WindowId, …>` |
| Custom title bar | [x] | Borderless + drag |
| Borderless resize | [x] | Kenardan sürükle |
| Occlusion sonrası repaint | [x] | `WindowEvent::Occluded` |
| Fullscreen toggle | [x] | F11 / menü / palet |
| Maximize / minimize | [x] | Title bar butonları |
| Always on top | [x] | `[window].always_on_top` |
| Pencere geometrisi kalıcılığı | [x] | Kapanışta kayıt |
| Native decorations seçeneği | [x] | `[window].decorations = "native"` |

### Detay checklist

- [x] Birden fazla pencere
- [x] Tab detach → yeni pencere
- [x] Son pencere kapanınca çıkış
- [x] Custom title bar (menü, kapat)
- [x] Title bar'dan pencere sürükleme
- [x] Kenar / köşeden resize
- [x] Debounced PTY resize (150ms)
- [x] `ScaleFactorChanged` desteği
- [x] Occluded → visible repaint
- [x] Fullscreen
- [x] Maximize / minimize butonları
- [x] Pencere boyut/konum kaydetme
- [x] Native OS decorations seçeneği

---

## 15. Zil (Bell)

| Özellik | Durum | Not |
|---------|:-----:|-----|
| Görsel bell (flash) | [x] | `bell_flash_until` |
| Sesli bell | [x] | `[bell].audible` + paplay/afplay |
| Urgency / taskbar flash | [x] | `[bell].urgency` + WM attention |
| Bell config | [x] | `[bell].visual` toggle |

### Detay checklist

- [x] `TerminalEvent::Bell` işleme
- [x] Görsel flash (150ms sarı overlay)
- [x] Config ile aç/kapat
- [x] Sistem sesi / custom ses
- [x] Urgency hint (WM attention)

---

## 16. Performans & Gözlemlenebilirlik

| Özellik | Durum | Not |
|---------|:-----:|-----|
| Perf overlay | [x] | frame / fps / capture / scene / gpu ms |
| Damage-aware capture | [x] | `FrameDamage` |
| Chrome scene cache | [x] | `chrome_scene_valid` |
| Skipped frame sayacı | [x] | Boş wakeup atlama |
| FPS geçmiş grafiği | [x] | Perf overlay sparkline |
| SIMD UTF-8 (yerel) | [x] | `--features simd-utf8`; OSC tap path + CI test matrisi |

### Detay checklist

- [x] Perf overlay toggle (menü / palet)
- [x] `frame_ms`, `fps`, `capture_ms`
- [x] `scene_ms`, `gpu_ms`
- [x] `skipped_frames` sayacı
- [x] Pane / dirty row istatistikleri
- [x] `needs_redraw` gate
- [x] Buffer reuse (`pane_frame_bufs`, `text_buf`)
- [x] Zaman serisi FPS grafiği
- [x] Profiler entegrasyonu (`tracing` scope + `RUST_LOG=sherion=trace`)
- [x] SIMD UTF-8 (`--features simd-utf8`, CI'da doğrulanır)

---

## İstatistik özeti

| Durum | Açıklama |
|-------|----------|
| `[x]` | Tam uygulanmış |
| `[~]` | Kısmen / sınırlı |
| `[ ]` | Uygulanmamış |

> Detay checklist'lerdeki maddeleri tamamladıkça `[ ]` → `[x]` olarak güncelleyin.
> Tüm özellik maddeleri tamamlandı; kalan `[~]` maddeler sağlama alma ile `[x]` yapıldı.

---

## Önerilen öncelik sırası (uyumluluk)

1. [x] Mouse reporting (SGR + 1002/1003)
2. [x] Bracketed paste
3. [x] DECCKM (application cursor keys)
4. [x] IME / dead key desteği
5. [x] Yapılandırılabilir keybinding'ler
6. [x] Strikethrough + dim render
7. [x] Gerçek split pane
8. [x] Full scrollback arama
9. [x] Sesli bell + fullscreen
10. [x] OSC 8 hyperlink

### Sonraki öncelikler

1. [x] modifyOtherKeys
2. [x] Ligatures (`[font] ligatures = true`)
3. [x] GPU glyph atlas (`[font] glyph_atlas = true`)
4. [x] Windows / ConPTY (portable-pty + cross-platform output tap)
5. [x] Profil / çoklu config (`--profile`, `SHERION_PROFILE`, `[profiles.*]`)
6. [x] Çok satırlı URL algılama + hover span
7. [x] Özel arka plan görseli
8. [x] Profiler (`tracing` hot-path scopes)
9. [x] Metin blink (vendor `alacritty_terminal` SGR 5 patch)
10. [x] Özel arka plan shader

### Sağlama alma (tamamlandı)

1. [x] Non-Unix busy heuristic — `src/pty/busy.rs` unit testleri
2. [x] Combining/zerowidth render — `src/render/scene.rs` regression testleri
3. [x] SIMD UTF-8 — CI'da `--features simd-utf8` test matrisi
