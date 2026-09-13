# MikroTik RIF Viewer — Русский
# Отсутствующие идентификаторы берутся из английского файла.

app-title = MikroTik RIF Viewer

## Экран приветствия
welcome-blurb = Откройте дамп поддержки RouterOS и просмотрите все содержащиеся в нём модули. Всё выполняется на вашем устройстве — ничего не загружается.
welcome-start = Начать

## Главный экран
drop-title = Перетащите сюда supout.rif
drop-title-hover = Отпустите файл, чтобы открыть его
drop-hint = или нажмите, чтобы выбрать файл · всё выполняется на вашем устройстве

## Анимация открытия
phase-reading = Чтение файла…
phase-indexing = Индексация модулей…
phase-preparing = Подготовка просмотра…
detail-indexing = Чтение структуры дампа
detail-read-of = { $done } из { $total }
detail-read = прочитано { $size }
detail-modules-found = { $count ->
    [one] найден { $count } модуль
    [few] найдено { $count } модуля
   *[many] найдено { $count } модулей
}

## Состояние и нижняя панель
status-ready = Готово
status-decoding = Декодирование модуля…
status-decoding-inline = Декодирование…
status-saved = Сохранено в { $path }
status-indexed = { $count ->
    [one] проиндексирован { $count } модуль
    [few] проиндексировано { $count } модуля
   *[many] проиндексировано { $count } модулей
}
status-copied = { $count ->
    [one] { $count } байт скопирован в буфер обмена
    [few] { $count } байта скопировано в буфер обмена
   *[many] { $count } байт скопировано в буфер обмена
}
footer-build = v{ $version } · { $os } · { $arch }

## Рабочая область
label-modules = Модули
label-line-numbers = Номера строк
label-no-capture = нет дампа
label-compressed = { $size } сжато
label-counter = { $position } / { $total }
label-copy-suffix = · копия { $count }
label-find-counter = { $position } / { $total }
empty-selection = Выберите модуль слева, чтобы увидеть его содержимое
empty-filter = Ни один модуль не соответствует запросу

## Подсказки ввода
hint-filter = Фильтр модулей
hint-find = текст в этом модуле

## Кнопки
button-open = Открыть…
button-copy = Копировать
button-save-as = Сохранить как…
button-previous = Предыдущее
button-next = Следующее
button-clear = Очистить
button-toggle-rail = Показать или скрыть список модулей
button-find = Поиск в этом модуле

## Ошибки и состояние модуля
module-unreadable = Не удалось проиндексировать этот модуль.
error-open = Не удалось открыть { $path }: { $reason }
error-module = Не удалось прочитать модуль { $index }: { $reason }
error-save = Не удалось сохранить: { $reason }

## Системные диалоги
dialog-open-title = Открыть дамп RouterOS
dialog-filter-name = Дамп RouterOS

## Обновления
update-available = Доступна версия { $version }
update-check = Проверить обновления
update-now = Обновить сейчас
update-later = Позже
update-skip = Пропустить эту версию
update-auto = Проверять обновления автоматически
