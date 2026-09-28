# طرح محصول: BiFlow CLI و MCP هوش‌مصنوعی‌محور

وضعیت: **طرح پیشنهادی برای بررسی و تبدیل مرحله‌ای به ADR و کار اجرایی**  
تاریخ تهیه: ۲۰۲۶-۰۹-۲۷  
بازبینی کیفیت: ۲۰۲۶-۰۹-۲۷  
دامنه: CLI و MCP محلی برای Ubuntu، Debian و Windows؛ TUI بعد از قرارداد JSON

## خلاصه

GUI می‌ماند. CLI و MCP همان محصول را روی همان runtime کنترل می‌کنند. سه نسخه
جدا از routing و lifecycle ساخته نمی‌شود. ترمینال اول باید فرمان غیرتعاملی و
JSON پایدار بدهد؛ داشبورد رنگی بعد از آن است، نه شرط شروع.

## درخواست صریح کاربر

- برابری قابلیت محصول در GUI و CLI، نه چند فرمان تشخیصی.
- عرضه برای Ubuntu 22.04 / 24.04، Debian، Windows 10/11. Ubuntu 26.04 فقط
  smoke است، نه مانع انتشار اول.
- GUI فعلی Windows و Linux بماند و با CLI/MCP یک runtime را سهم کند.
- help کامل؛ انسان و اسکریپت هر دو.
- MCP تا عامل بتواند «Windscribe را وصل کن و پیش‌فرضش کن» را اجرا کند، خطا را
  ببیند و فقط تعمیر از قبل کدشده را پیشنهاد دهد.
- TUI خوانا، بعد از این‌که همان کار از فرمان JSON ممکن شد.
- تنظیمات موفق‌اعمال‌شده بین GUI و CLI قابل جابه‌جایی باشد: export از یکی،
  import در دیگری؛ همان بسته در TUI و MCP.

## بایدهای مهندسی

این‌ها شرط طرح‌اند، نه فهرست ایده‌ها:

1. **یک مالک stack.** دو `Engine` هم‌زمان روی TUN، route یا helper ممنوع است.
   مالک یا `biflowd` کاربر است یا خود GUI با IPC به آن. انتخاب در فاز صفر با
   prototype روی Linux و Windows ثابت می‌شود و یک ADR می‌گیرد.
2. **CLI به helper ممتاز وصل نمی‌شود.** helper لینوکس root و helper ویندوز
   SYSTEM فقط از runtime مالک صدا می‌شوند. CLI pipe یا سوکت helper را باز
   نمی‌کند و دستور sudo/PowerShell نمی‌سازد.
3. **موفقیت یعنی وضعیت زنده.** بعد از set-default یا Connect، `GET /rules`
   برای `MATCH` و در صورت side-tunnel بودن، egress probe باید با config ارسالی
   بخواند. ذخیره در `config.toml` به‌تنهایی موفق نیست.
4. **خروجی ماشین قرارداد است.** `--output json`، `schema_version`، کد خطای
   پایدار، `trace_id`، `operation_id`. stdout فقط payload؛ پیشرفت و log روی
   stderr. MCP هم همین قاعده را برای JSON-RPC دارد.
5. **نام باینری جدا.** `biflow-cli` / `biflow-cli.exe` در PATH؛ با `BiFlow.exe`
   قاطی نشود.
6. **منطق از Tauri جدا، رفتار GUI ثابت.** commandهای نازک می‌مانند. استخراج
   باید `tauri-plugin-single-instance`، tray، `BIFLOW_DEV_PROFILE` و پورت‌های
   کنترلر dev/production را نگه دارد. فلگ `--profile` جدید ساخته نمی‌شود؛
   همان متغیر فعلی پروفایل را مشخص می‌کند.
7. **عمر process در ویندوز یک تصمیم است، نه سه محصول.** بعد از prototype یکی
   انتخاب می‌شود: daemon session بعد از بستن GUI زنده می‌ماند، یا CLI صریحاً
   می‌گوید runtime در دسترس نیست. هر سه حالت on-demand و login و وابسته به
   GUI با هم ship نمی‌شوند.
8. **لینوکس دو process می‌ماند.** `systemd --user` برای daemon کاربر؛ helper
   root فعلی دست نخورده. polkit/مسیر نصب باید روی Debian و Ubuntu یک رفتار
   داشته باشد، بدون نام پکیج فقط اوبونتو.
9. **کف glibc Ubuntu 22.04 amd64 است.** Debian 12/13 از همان باینری/.deb
   استفاده می‌کنند مگر dependency اسم متفاوت داشته باشد که در بسته اعلام
   شود. arm64 و WSL هدف نیستند؛ `doctor` روی WSL همان را بگوید.
10. **MCP فقط stdio و ابزار دامنه.** نه shell، نه خواندن مسیر دلخواه بیرون از
    home/پروفایل برای بستهٔ تنظیمات، نه کنترلر Mihomo، نه HTTP. mutation بعد از
    فرمان JSON معادل. recovery فقط `action_id`هایی که از قبل در محصول وجود
    دارند (retry تونل، probe، fresh-hiddify). مدل repair جدید اختراع نمی‌کند.
11. **برش اول (MVP) قبل از TUI و MCP نوشتنی.** `doctor`، `status`،
    `connect` / `disconnect` / `pause` / `resume`، `client list` /
    `set-default` / `connect`، `route test`، `client probe`، JSON و exit
    code. باقی ماتریس parity بعد از این برش است.
12. **یک بستهٔ تنظیمات، چهار سطح.** GUI، CLI، TUI و MCP همان سرویس
    export/import را صدا می‌زنند. محتوا همان سندهای موجود است (`config.toml`
    با مهاجرت schema فعلی، سند پین‌ها، کپی پروفایل‌های محلی اشاره‌شده)، نه
    فرمت موازی. فقط آخرین persist موفق بعد از apply صادر می‌شود، نه پیش‌نویس
    اعمال‌نشده. import همان `validate` و مسیر save/apply گرافیکی است. MCP
    فایل را می‌نویسد/می‌خواند و در پاسخ ابزار خلاصهٔ redacted می‌دهد، نه
    secret و نه کل سند.

## عمداً ساخته نمی‌شود

برای جلوگیری از over-engineering این‌ها از دامنهٔ اجرا خارج‌اند مگر ADR جدا:

- پروتکل MCP از صفر، transport شبکه‌ای، یا generator تنظیمات هر host
- musl/static، AppImage مخصوص CLI، یا ماتریس arm64
- completion/man به‌عنوان گیت انتشار اول
- API جدا برای TUI
- لایهٔ مجوز پیچیده‌تر از سه scope مشاهده / کنترل شبکه / نصب سیستم
- registry recovery که عمل جدیدی غیر از GUI داشته باشد
- فرمت تنظیمات دوم، همگام‌سازی ابری، یا برگرداندن secret کامل داخل نتیجهٔ MCP

## وضعیت فعلی مخزن

- `crates/iran-split-cli` محصول نیست: `demo`، `validate-config`، `route`
  آفلاین، `probe`. `demo` backend نمایشی است.
- GUI از `apps/desktop/src/api/desktop.ts` به `src-tauri/src/lib.rs` وصل است.
- engine در `iran-split-core` است؛ backendها
  `iran-split-platform-linux` و `iran-split-platform-win`؛ helper از
  `iran-split-ipc`.
- CLI که خودش Engine را روشن کند با GUI بر سر TUN رقابت می‌کند.

## تصمیم معماری فاز صفر

تصمیم فاز صفر: process خود BiFlow مالک Engine می‌ماند؛ GUI و CLI روی IPC
محلی همان process کار می‌کنند. روی Linux سوکت Unix در runtime directory کاربر
با mode `0600` است. روی Windows named pipe کاربر با endpoint جدا برای هر
username است. `BIFLOW_DEV_PROFILE` endpoint جدا می‌گیرد. CLI هرگز Engine یا
helper دومی نمی‌سازد. daemon per-user به فاز یک موکول می‌شود تا از همین
قرارداد و تست‌های ویندوز/لینوکس استفاده کند.

Prototype باید فقط این‌ها را جواب بدهد:

- یک engine، دو client هم‌زمان، بدون TUN تکراری؛ روی Linux و Windows.
- بستن ترمینال، Connect را لغو یا دوباره اجرا نمی‌کند؛ CLI می‌تواند operation
  را از `operation_id` پیگیری کند.
- بستن پنجرهٔ اصلی GUI آن را به tray می‌فرستد و CLI همچنان به runtime متصل
  می‌شود. پس از Quit کامل برنامه، CLI پیام روشن «runtime در دسترس نیست» می‌دهد.
- helper ممتاز همان پروتکل فعلی است؛ daemon کاربر جای UAC/polkit نمی‌نشیند.

اگر daemon روی یک پلتفرم شکست، همان پلتفرم IPC به GUI موجود را prototype
می‌کند. دو runtime هم‌زمان پذیرفته نیست.

## نقشه‌ی اسناد

- [ماتریس برابری و قرارداد فرمان‌ها](./feature-parity.md)
- [معماری runtime، ترمینال و MCP](./architecture-and-mcp.md)
- [مراحل، گیت‌ها و تست](./roadmap-and-acceptance.md)

## مراجع

- GUI: `apps/desktop/src/api/desktop.ts`، `src-tauri/src/lib.rs`
- engine: `crates/iran-split-core`
- backend: `crates/iran-split-platform-linux`، `crates/iran-split-platform-win`
- helper: `crates/iran-split-ipc`، `crates/iran-split-helper`
- MCP: [Specification 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28)
- OS: [Ubuntu](https://ubuntu.com/about/release-cycle)،
  [Debian](https://www.debian.org/releases/)،
  [Windows 10](https://learn.microsoft.com/en-us/windows/release-health/release-information)،
  [Windows 11](https://learn.microsoft.com/en-us/windows/release-health/windows11-release-information)

## بیرون از دامنهٔ این پوشه

این سند کد، نصب‌کننده یا انتشار را عوض نمی‌کند. اجرا بعد از ADR فاز صفر است.
