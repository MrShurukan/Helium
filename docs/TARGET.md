# Текущее описание target

B2 централизует поддерживаемую часть ATmega328P в
`src/backend/microcontroller.rs`. Это описание возможностей микроконтроллера,
не разводки Nano и не универсальная модель всей периферии.

## Что использует описание

- Source support, IR verifier и LLVM emitter проверяют цифровые пины по одной маске.
- LLVM emitter получает отдельные data-space адреса PIN/DDR/PORT, регистры Timer1,
  маски CTC/IRQ/W1C, вектор обработчика и I/O-space адрес SREG для IN/OUT.
- Планировщик использует единые делители и предел счётчика; verifier проверяет
  конфигурацию через тот же планировщик. Clock limit и текст диагностики согласованы.
- LLVM triple и имя MCU для opt/llc/avr-gcc, а также имя для avrdude находятся
  в том же описании. timer-options проверяет аппаратное имя по описанию.

Порядок запуска остаётся в emitter: остановить таймер и замаскировать IRQ,
настроить режим, записать старшие байты перед младшими, очистить флаги прямой
W1C-записью, разрешить сравнение и запустить таймер. B2 не меняет способ исполнения
GPIO и не ослабляет shared-access проверки.

B3 добавляет ёмкости Flash/SRAM и начало SRAM для отчёта итогового ELF; см.
[MEMORY_AND_TIMING.md](MEMORY_AND_TIMING.md). Резерв bootloader пока не учитывается.

## Границы

Поддерживается один ATmega328P и один Timer1 CTC. R02 выполнен: CLI выбирает descriptor
один раз; support, planner, IR, verifier, emitter, toolchain и memory report получают его.
Встроенный target выбирается по имени только на входной границе. Внешний файл пока
загрузить нельзя; R03/R04 сохраняются.

`compile_llvm`/`compile_typed_ir` сохраняют вход с именем MCU. Варианты
`compile_llvm_with_target`/`compile_typed_ir_with_target` принимают уже выбранный descriptor;
он должен описывать поддерживаемую модель оборудования и жить весь процесс.
Это Rust API, не обещание проверки произвольного внешнего файла. IR сохраняет ссылку
на descriptor; TimerConfig — на описание таймера. Verifier отвергает их рассогласование.
`timer::{plan,plan_with_direction,candidates,validate_clock}` и `llvm_toolchain::build`
теперь требуют descriptor явно. Для raw LLVM caller передаёт соответствующий target;
второго скрытого выбора ATmega328P в build нет. Загрузка/валидация внешних описаний — R03.

Маски B/D: 0..7, C: 0..6 отражают существующие цифровые биты, а не гарантию их
доступности на конкретной плате: альтернативные функции, clock/fuse/reset и разводка
должны учитываться при выборе выводов. Компилятор не меняет fuses. Верхний предел
CPU clock в текущем API также не доказывает допустимость частоты при любом питании.

Адреса GPIO/Timer1, маски и номер вектора сверены с `avr/iom328p.h` установленного
AVR toolchain; сведения о GPIO доступны в
[даташите Microchip ATmega328P](https://ww1.microchip.com/downloads/en/DeviceDoc/Atmel-7810-Automotive-Microcontrollers-ATmega328P_Datasheet.pdf).
Заголовки toolchain не требуются для сборки самого Helium и не загружаются при компиляции.

## Проверка B2 — 27 октября 2026

R02: полный gate прошёл с 143 тестами в каждом профиле. Три HEX (`llvm-gpio`,
`timer-seconds`, `functions-thermostat`, size/16 MHz) совпали побайтно с состоянием
до R02; локальные артефакты — build/review-before и build/review-r02.
Тестовое изменённое описание проверяет clock/timer/vector и отказ по переданной
ёмкости Flash; оно не добавлено в каталог поддерживаемых MCU.

`tools/test-all.ps1`: fmt, Clippy, 126 тестов в debug и 126 в release,
включая машинные тесты AVR8js. Новые проверки охватывают границы GPIO во всех трёх
стадиях, независимые адреса PIN/DDR/PORT, делители, предел Timer1, кодировку и порядок
записи регистров. Прямой LLVM emitter теперь также отвергает C7.

Контрольные сборки до/после B2: ATmega328P, CPU 16 MHz, `--opt size`, один toolchain.
Сравнивались полные HEX; все три совпали побайтно.

| Пример | .text, байт | .data, байт | .bss, байт |
| --- | ---: | ---: | ---: |
| llvm-gpio | 170 | 0 | 0 |
| timer-seconds | 328 | 0 | 4 |
| functions-thermostat | 140 | 0 | 0 |

Стек не включён. Это размеры конкретных примеров, а не оценка будущего приложения.
Артефакты локального сравнения: `build/b2-before`, `build/b2-after`; лог полного
прогона: `build/b2-full.log`. На физическую плату в этой итерации ничего не прошивалось.

## Дополнение B2a

Текущий emitter использует явные SBI/CBI для DDR/PORT. Перевод data-space в low-I/O
проверяет atomic_bit_io; неподдержанный адрес не получает RMW fallback. Контракт
независимости пинов и обновлённые результаты — [ATOMIC_GPIO.md](ATOMIC_GPIO.md).
