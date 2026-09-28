# نقشه‌ی اجرا، گیت‌ها و پذیرش

هر فاز قابل merge است و GUI را خراب نمی‌کند. برآورد روز نیست. TUI و MCP نوشتنی
پشت MVP فرمان‌های JSON می‌مانند.

## فاز 0 — prototype مالک runtime

**هدف:** یک engine، GUI و CLI هم‌زمان؛ بدون TUN تکراری.

- فهرست capability از `desktop.ts` / Tauri فقط برای برش MVP و ثبت بقیه در
  جدول parity؛ کاتالوگ کامل قبل از prototype لازم نیست.
- process GUI مالک Engine است و local IPC روی Linux و Windows `status` و
  `connect` را ارائه می‌دهد. endpoint و lifecycle در ADR ثبت می‌شوند.
- بستن پنجرهٔ GUI آن را به tray می‌فرستد؛ CLI همچنان کار می‌کند. پس از Quit
  کامل، CLI پیام روشن «runtime نیست» می‌دهد.
- helper همان پروتکل فعلی؛ CLI به آن وصل نشود.
- MCP در این فاز لازم نیست.

**گیت:** یک process مالک stack؛ GUI و دو درخواست CLI هم‌زمان؛ بستن ترمینال
Connect را لغو یا دوباره اجرا نمی‌کند؛ `status` بعد از reconnect همان
`operation_id` را می‌بیند. Native Windows و Linux builds موفق‌اند. ADR تصمیم
مالکیت و endpoint را ثبت می‌کند.

## فاز 1 — سرویس مشترک، GUI سبز

- orchestration را از Tauri به crate مشترک ببر؛ commandها نازک بمانند.
- single-instance، tray، `BIFLOW_DEV_PROFILE` و پورت کنترلر را regression نده.
- لینوکس: daemon کاربر جدا از helper root. ویندوز: pipe کاربر جدا از SYSTEM؛
  یک استراتژی عمر process از فاز صفر.
- مسیر config/log را بی‌دلیل عوض نکن.

**گیت:** Connect/Pause/Resume/Disconnect از GUI لینوکس و ویندوز سبز؛ Clippy و
تست crateهای دست‌خورده بدون هشدار.

## فاز 2 — CLI MVP

`doctor`, `status`, `connect`, `disconnect`, `pause`, `resume`,
`client list`, `client set-default`, `client connect`, `route test`,
`client probe`؛ `--output json`، exit code، `--non-interactive`، `--no-color`.

بلافاصله بعد از MVP، همان سرویس: `settings export` / `import` تا GUI بتواند
بستهٔ CLI را بخواند و برعکس. سپس بقیهٔ ماتریس.

**گیت MVP:** pipe و SSH بدون TTY گیر نمی‌کنند؛ JSON در خطا معتبر است؛ set-default
بدون MATCH زنده موفق گزارش نمی‌شود. گیت بستهٔ تنظیمات: export بعد از apply
موفق در CLI، import در GUI همان default و کلاینت‌ها را نشان می‌دهد؛ مسیر
برعکس هم با یک تست round-trip. parity کامل گیت انتشار نهایی است نه این فاز.

## فاز 3 — MCP خواندنی

همان باینری، `mcp serve --stdio`. tools: status، clients، operation،
diagnostics، route test، probe، logs redactشده. schema در repo و تست stdout
خالص (به‌خصوص path با فاصله روی Windows).

**گیت:** عامل می‌تواند بگوید Windscribe بالا نیست و `trace_id` / evidence بدهد؛
روی stdout قبل از protocol چیزی نباشد.

## فاز 4 — MCP کنترل

mutation فقط بعد از فرمان JSON معادل. connect / set-default با `operation_id`.
`settings_export` / `settings_import` بعد از همان فرمان‌های CLI. پاسخ ابزار
خلاصهٔ redacted است. `apply_recovery_action` فقط action موجود GUI. بدون
command اختراعی.

**گیت:** سناریوی Windscribe در محیط آزمایش: set-default، connect، MATCH زنده،
egress probe. اگر probe رد شد، خطا همان است نه «موفق».

## فاز 5 — TUI

داشبورد روی همان client فرمان‌ها. export/import تنظیمات از TUI همان بستهٔ GUI
و CLI است. رنگ به‌تنهایی معنا نیست. اگر bidi فارسی در جدول خراب بود، شناسه‌ها
LTR؛ layout جدید نه. screenshot دو اندازه کافی است، ماتریس تمام ترمینال‌ها
لازم نیست.

**گیت:** TUI در pipe بالا نمی‌آید؛ MCP را آلوده نمی‌کند.

## فاز 6 — بسته و ماتریس OS

### لازم برای اعلام پشتیبانی

| سیستم                    | نقش                                           |
| ------------------------ | --------------------------------------------- |
| Ubuntu 22.04 amd64       | کف glibc                                      |
| Ubuntu 24.04 amd64       | بستهٔ اصلی لینوکس                             |
| Debian 12 amd64          | حد پایین Debian                               |
| Debian 13 amd64          | Debian جاری                                   |
| Windows 10 22H2 x64      | سازگاری برنامه، نه پشتیبانی امنیتی مایکروسافت |
| Windows 11 x64 supported | هدف ویندوز                                    |

Ubuntu 26.04 فقط smoke است. arm64 و WSL پشتیبانی نمی‌شوند؛ `doctor` روی WSL
بگوید.

CLI به GTK/WebView2 وابسته نیست. `.deb` نام پکیج فقط اوبونتو ندارد.
`biflow-cli.exe` جدا از `BiFlow.exe`. helper ویندوز همان scheduled task
SYSTEM. نصب CLI برای `status`/`doctor` UAC نمی‌خواهد.

CI روی ردیف‌های لازم: نصب پاک، `doctor`، `status`، uninstall. Connect واقعی و
OpenVPN روی VM بدون درایور با خطای تشخیصی کافی است، نه رد شدن کل ماتریس.

## تست؛ فقط آنچه جلوی برگشت را می‌گیرد

- unit: exit code، redaction، JSON خطا، revision conflict.
- IPC: UID لینوکس، SID/ACL ویندوز، فریم خراب، timeout، cancel.
- قرارداد: command جدید GUI بدون ردیف CLI/MCP در جدول parity رد شود.
- process CLI: `--help`، آرگومان بد، pipe، `NO_COLOR`، SIGINT.
- MCP: stdout خالص، بدون secret، بدون exec دلخواه.
- Windscribe آزمایشگاهی وقتی helper و TAP در دسترس‌اند: MATCH و egress.

completion و man گیت انتشار اول نیستند.

## تحویل

- MVP فاز ۲ روی ماتریس OS لازم سبز است.
- parity کامل وقتی جدول، help و تست هر ردیف را دارد.
- MCP خواندنی قبل از MCP کنترل؛ کنترل قبل از TUI.
- docs: نصب، quickstart، quoting MCP در Windows/Linux، scope، این‌که WSL
  پشتیبانی نمی‌شود.
