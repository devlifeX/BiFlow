# ماتریس قابلیت‌ها و قرارداد فرمان‌های CLI

نام فرمان‌ها پیشنهادی‌اند و قبل از پیاده‌سازی در ADR تثبیت می‌شوند. سلول‌های
این جدول `|` خام ندارند تا قرارداد قابل خواندن بماند.

## تعریف parity

هر کار محصولی GUI باید از فرمان قابل انجام باشد. صفحه‌بندی، انیمیشن و tray
معادل CLI لازم ندارند. هر capability در help، `--non-interactive`، در صورت
امکان JSON، exit code، همان application service، و در صورت مناسب بودن MCP
می‌آید.

**برش اول (MVP)** فقط ردیف‌های Dashboard، اتصال کلی، client list/set-default/connect،
default route، diagnostics probe/route test است. بلافاصله بعد از آن، ردیف
تنظیمات شامل `export` / `import` تا GUI و CLI یک بسته را رد و بدل کنند. TUI
فرمان جدا ندارد؛ همان `settings export` و `import`.

## ماتریس GUI به CLI و MCP

| حوزه          | CLI                                                                                                                            | MCP                                                                                                       | نکته                                                               |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| وضعیت         | `status`, `watch`, `traffic`, `connections list`                                                                               | `biflow_status`, `connections_list`                                                                       | helper، Mihomo، TUN، DNS، کلاینت‌ها و MATCH زنده؛ فاز typed نه رنگ |
| اتصال کلی     | `connect`, `disconnect`, `pause`, `resume`, `restart`, `operation show`, `operation cancel`                                    | `connect`, `disconnect`, `pause`, `resume`, `operation_get`, `operation_cancel`                           | پاسخ start حتماً `operation_id`؛ poll همان ID؛ retry کور ممنوع     |
| کلاینت‌ها     | `client list`, `show`, `add`, `remove`, `enable`, `disable`, `connect`, `disconnect`, `set-default`, `profile import`, `probe` | `clients_list`, `client_get`, `client_connect`, `client_disconnect`, `client_set_default`, `client_probe` | پروفایل از path محلی؛ credential چاپ نشود                          |
| مسیر پیش‌فرض  | `route default show`, `route default set CLIENT_OR_DIRECT`, `route test HOST`                                                  | `default_route_get`, `default_route_set`, `route_test`                                                    | بعد از apply، MATCH زنده از کنترلر؛ ذخیره تنها کافی نیست           |
| قوانین DIRECT | `rules direct list`, `add`, `remove`, `refresh`                                                                                | `direct_rules_list`, `add`, `remove`, `refresh`                                                           | mutation با revision                                               |
| پین میزبان    | `route pin HOST --via CLIENT_OR_DIRECT`, `route unpin`, `route pins reassign`, `discard`                                       | `route_pin`, `route_unpin`, `client_pins_reassign`, `discard`                                             | همان قواعد PSL و companion موجود                                   |
| فهرست سفارشی  | `rules list create`, `rename`, `delete`, `set-route`, `add`, `remove`, `check`, `pin`                                          | `rule_lists_*`, `rule_list_check`                                                                         | نتیجه per-entry                                                    |
| cloud rules   | `rules cloud status`, `sync`                                                                                                   | `cloud_rules_status`, `sync`                                                                              | شکست، snapshot قبلی را نگه دارد                                    |
| تنظیمات       | `settings show`, `validate`, `set`, `apply`, `export PATH`, `import PATH`                                                      | `settings_get`, `validate`, `update`, `apply`, `settings_export`, `settings_import`                       | یک بسته برای GUI/CLI/TUI/MCP؛ فقط persist موفق؛ MCP بدون secret    |
| تشخیص         | `diagnose`, `diagnose route TARGET`, `reachability`, `client probe`, `config running` redactشده                                | `diagnostics_run`, `route_test`, `reachability_check`, `egress_probe`                                     | `trace_id`؛ YAML خام و secret کنترلر بیرون نرود                    |
| وابستگی       | `deps list`, `install`, `guide`, `helper status`, `helper install`, `hiddify fresh-start`                                      | `dependencies_*`, `helper_status`, `helper_install`, `hiddify_repair`                                     | elevation همان GUI؛ CLI خودش root نمی‌شود                          |
| لاگ           | `logs status`, `tail`, `query`, `export`, `delete`, `support export PATH`                                                      | `logs_query`, `debug_log_status`, `support_bundle_export`                                                 | redact قبل از serialize                                            |
| به‌روزرسانی   | `version`, `update check`, `status`, `install`                                                                                 | `update_status`, `check`, `install`                                                                       | همان تأیید امضا و asset دقیق                                       |
| کاتالوگ       | `client catalog`, `client profile import PATH`                                                                                 | `client_catalog`, `profile_import`                                                                        | URL فقط از allowlist کاتالوگ                                       |
| ظاهر          | `--language en` یا `fa`, `--color auto` یا `always` یا `never`                                                                 | نیست                                                                                                      | متن status بدون رنگ فهمیده شود                                     |
| tray و پنجره  | `status` / `watch`؛ TUI بعد از MVP                                                                                             | نیست                                                                                                      | tray و ناوبری GUI می‌مانند                                         |

`CLIENT_OR_DIRECT` یعنی شناسهٔ کلاینت یا کلمهٔ `direct`.

## شکل فرمان

```text
biflow-cli doctor
biflow-cli status --output json
biflow-cli client list --output json
biflow-cli client set-default windscribe --wait --timeout 60
biflow-cli connect --wait --timeout 60
biflow-cli route test example.com --output json
biflow-cli client probe
biflow-cli settings export ./biflow-settings
biflow-cli settings import ./biflow-settings
biflow-cli mcp serve --stdio
```

بدون TTY، اجرای خالی TUI باز نمی‌کند؛ help می‌دهد.

فلگ‌های عمومی: `--output human` یا `json`؛ `--no-color` و `NO_COLOR` /
`TERM=dumb`؛ `--non-interactive`؛ `--timeout` و `--wait`؛ `--revision` برای
mutation؛ `--yes` فقط confirmation محصول، نه دور زدن polkit/UAC. پروفایل
اجرایی همان `BIFLOW_DEV_PROFILE` است.

## envelope

```json
{
  "schema_version": 1,
  "ok": false,
  "request_id": "uuid",
  "operation_id": "uuid",
  "data": null,
  "error": {
    "code": "CLIENT_NOT_READY",
    "message": "Windscribe is enabled but its OpenVPN tunnel is not ready.",
    "retryable": true,
    "trace_id": "uuid",
    "evidence": [{ "check": "egress_probe", "result": "gateway_unreachable" }],
    "suggested_actions": ["client_probe", "diagnostics_run"]
  }
}
```

## Exit code

|  کد | معنی                                |
| --: | ----------------------------------- |
|   0 | موفق                                |
|   2 | آرگومان یا schema نامعتبر           |
|   3 | وضعیت محصول ناسازگار                |
|   4 | helper یا مجوز در دسترس نیست        |
|   5 | timeout یا readiness/egress ناموفق  |
|   6 | conflict revision یا عملیات هم‌زمان |
|   7 | ذخیره/IO                            |
|   8 | تأیید dependency یا بسته            |
| 130 | Ctrl-C با cancel همان operation     |

کدها مرکزی‌اند؛ از متن خطای Rust حدس زده نمی‌شوند.

## تکمیل parity

هر ردیف بعد از MVP: نام فرمان، owner، schema، تست، نمونهٔ help. mutation GUI و
CLI یک مسیر reload دارند.
