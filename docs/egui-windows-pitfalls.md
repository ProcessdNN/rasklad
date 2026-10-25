# rasklad: разбор шишек при разработке UI на egui/eframe (Windows)

Документ-источник для вливания в скиллы/регламенты. Всё ниже — проверено на живом
коде и замерах, а не «по общим соображениям». Стек и версии — фактические из проекта.

---

## 0. Стек проекта (факт)

| Слой | Что |
|---|---|
| Язык | Rust 2021, toolchain `stable-x86_64-pc-windows-msvc` |
| UI | `eframe 0.31.1` + `egui 0.31.1`, бэкенд **glow** (OpenGL), `default-features = false, features = ["default_fonts", "glow"]` |
| Окна/ввод | `winit 0.30.13` (транзитивно), `glutin 0.32.3` |
| Трей | `tray-icon 0.20.1` (внутри `muda 0.16.1`) |
| Win32 | `windows 0.61.3`, `winreg 0.52` |
| Конфиг | `serde` + `serde_json`, `%APPDATA%\rasklad\config.json` |
| Платформа | Windows-only (`#![cfg(windows)]` на модулях), tray-приложение |
| Архитектура | 2 потока: UI (eframe/winit event loop) + worker (свой `GetMessageW` цикл: `WH_KEYBOARD_LL` хук + трей) |

Особенность архитектуры: GUI-поток и worker общаются через `mpsc` + `PostThreadMessageW`,
состояние — в `Arc<Shared>` (Mutex + атомики). Окно настроек закрывается не в выход,
а в трей (`ViewportCommand::CancelClose` + `Visible(false)`).

---

## 1. Шишка №1 — попап меню сжимается в «палочки» (одна буква на строку)

### Симптом
Меню, привязанное к узкой кнопке (например `"…"`), рисуется колонкой из отдельных
букв: `A / ut / os / ta / rt`. Плюс обрезается краем окна.

### Причина (код egui 0.31.1, `src/containers/popup.rs`)
```rust
// popup_above_or_below_widget()
let frame = Frame::popup(parent_ui.style());
let frame_margin = frame.total_margin();
let inner_width = (widget_response.rect.width() - frame_margin.sum().x).max(0.0);
...
Area::new(popup_id)
    .fixed_pos(pos)
    .default_width(inner_width)      // ← ширина попапа = ширина виджета!
    .show(ctx, |ui| {
        frame.show(ui, |ui| {
            ui.with_layout(Layout::top_down_justified(Align::LEFT), |ui| {
                ui.set_min_width(inner_width);
                add_contents(ui)
            })
        })
    });
```
Кнопка `"…"` — это ~24 pt. Минус марги рамки → **~12 pt контента**. При
`top_down_justified` каждый `Button` растягивается на эти 12 pt, текст переносится
по буквам.

Вторая часть: попап — это `Area`, которая рисуется **в поверхности того же окна**.
За границы окна она выйти не может физически, `constrain(true)` прижимает её к
`screen_rect` = контенту окна.

### Как правильно
- Никогда не вешать попап на узкий виджет без явной ширины:
  ```rust
  popup_below_widget(ui, id, &resp, close, |ui| {
      ui.set_min_width(160.0);   // иначе унаследует ширину кнопки
      ...
  });
  ```
- Ширину меню считать по метрикам текста, а не угадывать:
  ```rust
  let text_w = ctx.fonts(|f| {
      ITEMS.iter()
          .map(|(_, label)| f.layout_no_wrap((*label).to_owned(), font.clone(), Color32::WHITE).size().x)
          .fold(0.0_f32, f32::max)
  });
  ```
- `Painter::text(...)` **не переносит** текст (внутри `layout_no_wrap`) — ручной
  рендер пунктов списка гарантированно не «сложится в палочки».
- `menu_button(...)` внутри `ui.with_layout(right_to_left, ...)` — тот же эффект:
  наследует узкий layout. Не использовать для меню в правом верхнем углу.

### Правило для скилла
> Любой popup/menu в egui: задавай ширину явно (`set_min_width` или расчёт по
> `layout_no_wrap`). Ширина по умолчанию = ширина виджета-якоря, а не контента.

---

## 2. Шишка №2 — отдельное OS-окно под меню: мигание и «вылет сбоку»

### Симптом
`ctx.show_viewport_immediate(...)` на клик: на долю секунды появляется пустое
прозрачное окно, меню «вылетает откуда-то сбоку», затем встаёт на место. Анимации нет.

### Причина
1. `show_viewport_immediate` создаёт **настоящее OS-окно** в момент клика. Первый
   кадр ещё не отрисован → видно пустую поверхность.
2. Позиция/размер применяются не мгновенно (см. шишку №3), плюс анимация открытия
   окна от DWM (если включена в системе) → «прыжок».
3. **Прозрачность в eframe задаётся глобально.** `glow_integration.rs`:
   ```rust
   let config_template_builder = glutin::config::ConfigTemplateBuilder::new()
       .with_transparency(native_options.viewport.transparent.unwrap_or(false));
   ```
   GL-конфиг создаётся **один раз для корневого viewport**. Значит дочернее окно
   нельзя сделать прозрачным, не сделав прозрачным всё приложение (а
   `DwmEnableBlurBehindWindow` на главном окне убивает скругления/тень Win11).
   → Плавный fade отдельного окна недостижим без костылей.

### Как правильно
Для меню/попапов **не создавать окно**. Рисовать внутри своего окна:
```rust
egui::Area::new(egui::Id::new("main-menu"))
    .order(egui::Order::Foreground)
    .fixed_pos(pos)          // считается от rect кнопки-якоря
    .movable(false)          // ВАЖНО: у Area по умолчанию movable = true!
    .show(ctx, |ui| {
        egui::Frame::popup(ui.style())   // margin/rounding/shadow из темы
            .show(ui, |ui| { /* пункты */ });
    });
```
Отдельное окно оставлять только там, где оно реально нужно (диалог настроек).

### Правило для скилла
> Меню/контекстное меню/дропдаун в egui = `Area` + `Order::Foreground` внутри
> текущего окна. Отдельный viewport под попап — источник мигания, прыжков и
> проблем с фокусом, и его нельзя анимировать прозрачностью (transparency глобальна).

---

## 3. Шишка №3 (главная) — новое окно создаётся размером «по умолчанию»

### Симптом
При открытии дочернего окна (Autostart / Advanced) на ~40 мс появляется **большое
пустое окно**, потом оно прыгает на нужный размер и место.

### Замер (опрос `EnumWindows` + `GetWindowRect` каждые 8 мс)
```
t=6496  Autostart  vis=True   52,52,1492,841      ← 1440×789 (!), пустое
t=6523  Autostart  vis=True   52,52,1492,841
t=6539  Autostart  vis=True   288,325,604,474     ← 316×149, правильное
```

### Причина (winit 0.30.13, `platform_impl/windows/window.rs`)
```rust
CreateWindowExW(
    ex_style, class_name, title, style,
    CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT,  // ← размер/позиция по умолчанию!
    parent, menu, hinstance, &mut initdata,
)
```
Окно создаётся **с размером по умолчанию** (в нашем случае 1440×789 — системный
дефолт, совпал с рабочей областью в логических точках при масштабе 133%), а
`with_inner_size`/`with_position` применяются **после** создания:
```rust
// egui-winit apply_viewport_builder_to_window()
window.request_inner_size(PhysicalSize::new(ppp * size.x, ppp * size.y));
window.set_outer_position(pos);
```
То есть `with_inner_size` / `with_position` в `ViewportBuilder` — **не гарантия**
размера на момент появления окна. Всё это время окно видимо (`with_visible(true)`
по умолчанию).

### Как правильно
Создавать скрытым и показывать, когда ОС реально применила геометрию:
```rust
#[derive(Default)]
struct ChildWindow { shown: bool, polls: u32 }

impl ChildWindow {
    fn ready(&mut self, ctx: &egui::Context, size: egui::Vec2) -> bool {
        if self.shown { return true; }
        self.polls += 1;
        let sized = ctx.input(|i| i.viewport().inner_rect)
            .map_or(false, |r| (r.size() - size).length() < 2.0);
        if sized || self.polls > 8 {          // 8 — safety-net
            self.shown = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            return true;
        }
        ctx.request_repaint();                // быстрый опрос, а не 200 мс
        false
    }
}

// использование
let builder = child_builder(ctx, "Autostart", [300.0, 110.0])
    .with_visible(self.autostart_win.shown);
ctx.show_viewport_immediate(id, builder, |ctx, _| {
    if !self.autostart_win.ready(ctx, size) { return; }   // не рисуем, пока не готово
    ...
});
```
Важные детали:
- `ViewportBuilder::patch()` превращает изменение `visible` в
  `ViewportCommand::Visible`, **без пересоздания** окна (recreate только для
  `active`, `close_button`, `app_id`, `clamp_size_to_monitor_size` и т.п.).
- `inner_rect` у скрытого окна всё равно обновляется (`GetClientRect` работает).
- Состояние `shown` сбрасывать при закрытии окна — immediate-viewport уничтожается,
  при следующем открытии окно создаётся заново.
- Результат замера после фикса:
  ```
  t=8643  Autostart  vis=False  210,247,526,396   ← создано скрытым, сразу нужный размер
  t=8661  Autostart  vis=True   210,247,526,396   ← показано через 18 мс, там же
  ```

### Правило для скилла
> На Windows (`winit`) новое окно **всегда** сначала появляется размером
> `CW_USEDEFAULT`. Если окно видимо при создании — пользователь увидит «большое
> пустое окно». Лечится: `with_visible(false)` + показ по факту нужного
> `inner_rect` (+ safety-counter). Не полагаться на `with_inner_size`/`with_position`
> как на «применится до показа».

---

## 4. Анимация появления меню (что работает в egui)

- У `Area` есть **встроенный fade-in**: при первом появлении `last_became_visible_at`
  и `ui.multiply_opacity(opacity)`, длительность = `style.animation_time`
  (по умолчанию 0.083 с), easing `quadratic_out`. Он сам вызывает `request_repaint`.
  → руками альфу писать не нужно, достаточно показывать/скрывать `Area`.
- `Area::new(id)` по умолчанию: `movable: true`, `constrain: true`, `fade_in: true`.
  Для меню обязательно `.movable(false)` — иначе карточку можно утащить мышью.
- Свой слайд (подъезд на 5 pt) — по времени:
  ```rust
  let t = ((ctx.input(|i| i.time) - opened_at) / style.animation_time as f64) as f32;
  let ease = 1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3);
  if t < 1.0 { ctx.request_repaint(); }
  ```
  `ctx.request_repaint()` внутри анимации — обязателен, иначе кадры не придут
  (в этом приложении базовый интервал перерисовки — 200 мс).
- `Frame::popup(style)` = `inner_margin = spacing.menu_margin`, `corner_radius =
  visuals.menu_corner_radius`, `shadow = visuals.popup_shadow`, `fill = window_fill`,
  `stroke = window_stroke`. Для ручного размера/позиции:
  `frame.total_margin().sum()` (возвращает `Vec2`).
- `Frame::multiply_with_opacity(p)` множит альфу fill + stroke + **shadow**.
- `Area` ограничена `screen_rect` окна → меню всегда влезает, если окно достаточно.
  Расчёт: окно 340×200, карточка меню ≈130×95 pt (шрифт 13.5, 3 пункта + разделитель)
  → помещается с запасом; в «elevated» режиме окно 340×140 — тоже.

---

## 5. Методика диагностики (самое ценное)

### 5.1 Поймать «мелькающее окно» без скриншотов
Опрос всех окон процесса с высоким разрешением по времени:
```
EnumWindows → GetWindowThreadProcessId == pid
            → GetWindowText / GetClassName
            → IsWindowVisible
            → GetWindowRect
```
Писать строку только при изменении состояния, `Start-Sleep -Milliseconds 8`.
Даёт точный ответ на вопросы «создаётся ли окно не того размера», «когда стало
видимым», «сколько это длится». Именно так была найдена шишка №3 (1440×789 → 316×149).

### 5.2 Снять содержимое GL-окна
`PrintWindow(hwnd, hdc, 2 /* PW_RENDERFULLCONTENT */)` — работает для OpenGL-окон
**даже если окно перекрыто другим приложением или сессия заблокирована**.
Ограничение: отдаёт статичный кадр — для покадровой съёмки анимации не годится
(все кадры будут побайтово одинаковыми).

### 5.3 Скриншот экрана
`Graphics.CopyFromScreen` — реальный композит (видно прозрачность/наложение), но
только для видимых окон и разблокированной сессии. Процесс-скрипт должен быть
DPI-aware (`SetProcessDPIAware`), иначе координаты окон и скриншота разъедутся.

### 5.4 Синтетический ввод
`PostMessage(WM_MOUSEMOVE/WM_LBUTTONDOWN/WM_LBUTTONUP)` **не работает** с winit/egui:
позиция указателя в egui берётся из реального состояния, клик не регистрируется.
Работает только реальный ввод: `SetCursorPos` + `mouse_event(LEFTDOWN/LEFTUP)`.
При этом:
- окно должно быть на переднем плане, иначе **первый клик съедается активацией**
  (нужен второй);
- `SetForegroundWindow` из фонового процесса Windows часто игнорирует;
- координаты считать от `GetClientRect` + `ClientToScreen`, а не на глаз
  (в нашем случае `(328, 15)` в клиентских координатах = кнопка `"…"`);
- курсор возвращать на место после теста.

### 5.5 Изоляция тестов
Приложение с single-instance мьютексом нельзя запустить рядом с рабочим экземпляром.
Схема: копия проекта во временную папку + замена имён мьютекса/события
(`rasklad-instance` → `rasklad-instance-test`) + отдельный `APPDATA` с безопасным
конфигом (`enabled: false`, хоткей на несуществующую клавишу `VK_F13 = 124`,
`caps_combo: Off`), чтобы не перехватывать Caps Lock пользователя.
Триггеры для теста — файл-маркер в temp (`Path::new("...\\go").exists()`), а не таймер:
таймер не синхронизируется с моментом, когда тестовый скрипт уже запущен.

---

## 6. Прочие специфики, которые всплыли

- **Single instance**: `CreateMutexW(None, true, w!("rasklad-instance"))` +
  `GetLastError() == ERROR_ALREADY_EXISTS`; второй экземпляр дёргает named event
  `rasklad-show`, чтобы первый показал окно. При elevated-relaunch нужен retry-цикл
  (старый процесс может ещё держать мьютекс).
- **Показ окна из трея**: Windows не даёт `SetForegroundWindow` фоновому процессу,
  поэтому после `ViewportCommand::Visible(true)` дополнительно поднимать окно нативно
  (`winshow::raise_main_window`), иначе окно только мигает в таскбаре.
- **Хук клавиатуры** (`WH_KEYBOARD_LL`): свои инъекции помечать `dwExtraInfo`
  (`LLKHF_INJECTED` + tag) и игнорировать; чужие инъекции (RDP/VM/AHK) обрабатывать
  как реальные. Ключ-триггер надо «глотать» и на keydown, и на keyup.
- **`schtasks`** запускать с `CREATE_NO_WINDOW` (0x0800_0000) — иначе консоль
  моргает и ворует фокус у дочернего окна.
- **`clear_color` в eframe — глобальный** (не per-viewport), через него нельзя
  сделать одно окно прозрачным.
- **debug vs release**: `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`
  → в debug-сборке есть консольное окно (мешает скриншотам, но видно `stderr`).
- **Профиль release** в проекте: `lto = "fat"`, `codegen-units = 1`, `opt-level = "z"`,
  `strip = true`. Полная сборка с нуля — ~1 минута, incremental debug — 8–20 с.
- **PATH**: `rustup --no-modify-path` не прописывает `~/.cargo/bin`; при установке
  тулчейна на чужой машине — либо без `--no-modify-path`, либо сразу дописать PATH.

---

## 7. Чек-лист «как не наступить» (для вставки в скилл)

1. Попап/меню в egui — **внутри окна** (`Area`, `Order::Foreground`, `.movable(false)`),
   ширина — явная, посчитанная по `layout_no_wrap`. Не привязывать к узкому виджету.
2. Любое **новое OS-окно** на Windows создаётся с `CW_USEDEFAULT` и ресайзится потом →
   создавать с `with_visible(false)`, показывать по факту нужного `inner_rect`,
   держать safety-counter.
3. `with_inner_size` / `with_position` — не гарантия геометрии на момент показа.
4. Прозрачность в eframe — глобальная. Не проектировать UI, где нужен
   полупрозрачный дочерний viewport.
5. Анимации в egui — `request_repaint()` каждый кадр, пока `t < 1`; длительность
   брать из `style.animation_time`, а не хардкодить.
6. Проверять UI-артефакты **замером** (EnumWindows/GetWindowRect с шагом ~8 мс,
   PrintWindow для контента), а не «на глаз по скриншоту».
7. Тестировать рядом с рабочим экземпляром — только через копию с другим мьютексом
   и отдельным `APPDATA`.
8. Синтетический ввод через `PostMessage` в winit/egui не доходит — только реальный.

---

## 9. Переключение раскладки и хуки (Win32-специфика)

### 9.1 Инжект «системного» хоткея переключения — ненадёжен

Первая версия `switch_layout()` инжектила через `SendInput` тот хоткей, который
«должен быть» настроен в системе (Alt+Shift по умолчанию). На тестовой машине
Alt+Shift и Ctrl+Shift **не делают ничего** — работает только Win+Space:

```
Alt+Shift   layout 67699721 -> 67699721   (без изменений)
Ctrl+Shift  layout 67699721 -> 67699721   (без изменений)
Win+Space   layout 67699721 -> 68748313   (переключилось)
```

То есть приложение «нажимало» клавиши, которые в системе отключены, и переключение
не работало вообще нигде — а выглядело как «капс переключается, а раскладка нет».

**Правильно:** не угадывать хоткей, а попросить окно переключиться напрямую:
```rust
// WM_INPUTLANGCHANGEREQUEST = 0x0050
let tid = GetWindowThreadProcessId(fg, None);
let next = next_layout(GetKeyboardLayout(tid));      // следующий из GetKeyboardLayoutList
PostMessageW(fg, WM_INPUTLANGCHANGEREQUEST, WPARAM(0), LPARAM(next.0 as isize));
```
- работает для любого окна, включая собственное (DefWindowProc применяет);
- не показывает оверлей языка (в отличие от Win+Space);
- **нести точный HKL, а не «следующий»** — тогда повторное применение не даёт
  двойного переключения;
- фолбэк `SendInput(Win+Space)` — только если `PostMessageW` вернул ошибку
  (UIPI: цель запущена с повышенными правами).

### 9.2 Колбэк хука нельзя блокировать

`switch_layout()` из колбэка `WH_KEYBOARD_LL` вызывать «в лоб» нельзя: внутри
проверка заняла бы десятки мс, а Windows снимает хук, если колбэк дольше
`LowLevelHooksTimeout` (~300 мс). Делать работу в отдельном потоке:
```rust
pub fn switch_layout() { std::thread::spawn(|| { if !via_message() { via_hotkey(); } }); }
```

### 9.3 UIPI: неэлевированный хук не видит ввод в элевированные окна

Low-level хук, установленный процессом без прав администратора, **не получает
события**, когда фокус в окне с повышенными правами. Отсюда классический симптом:
«в обычных окнах капс перехватывается, а в админских — капс просто переключается
и работает любым сочетанием». Лечится только запуском приложения от админа
(в проекте есть «relaunch as admin» и предупреждение в UI).

### 9.4 Захват хоткея: модификатор — это не клавиша

Наивная реализация «запомнить последнюю нажатую клавишу, по отпусканию — готово»
записывает `Alt` / `Win` / `Ctrl` как самостоятельный хоткей (и такой хоткей
съедает вообще все нажатия этой клавиши). Правила:
- при нажатии модификатора — только обновлять состояние модификаторов, **не**
  делать его кандидатом;
- кандидат — первая не-модификаторная клавиша; модификаторы берутся из состояния
  на момент её нажатия;
- валидация: модификатор сам по себе — нельзя; обычная клавиша (буква, цифра,
  Tab, Space) без модификатора — нельзя; F1–F24, Home/End/PgUp/PgDn/Insert/Delete,
  стрелки, PrintScreen, Pause, ScrollLock, NumLock, медиа-клавиши, Caps — можно
  без модификатора;
- невалидную комбинацию не «молча игнорировать»: вернуть причину в UI и
  **продолжить** захват (`CaptureResult::Rejected(&str)`), иначе пользователь
  думает, что приложение зависло.

---

## 10. Куда это вливать в регламенты

- `ai/roles/stacks/rust.md` — общие правила: `Cargo.lock` синхронизировать,
  debug/release отличия, `--no-modify-path`, стоимость `lto=fat`.
- новый `ai/roles/stacks/egui.md` (или `rust-egui.md`) — разделы 1–4 целиком:
  popup width, отдельные окна, `CW_USEDEFAULT`, `Area`/`Frame::popup`, анимации.
- `ai/AGENTS.md` / `tech-memory.md` — разделы 6 и 9 (Win32-специфика: мьютекс, хук,
  `CREATE_NO_WINDOW`, `raise_main_window`, переключение раскладки, UIPI).
- раздел 5 (методика) — в общий «как отлаживать UI», он не про egui, а про подход.

