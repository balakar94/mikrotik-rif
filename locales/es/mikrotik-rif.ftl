# MikroTik RIF Viewer — Español
# Los identificadores que falten aquí se toman del fichero inglés.

app-title = MikroTik RIF Viewer

## Pantalla de bienvenida
welcome-blurb = Abre una captura de soporte de RouterOS y lee todos los módulos que contiene. Todo se procesa en tu equipo: no se sube nada.
welcome-start = Empezar

## Inicio
drop-title = Arrastra aquí tu supout.rif
drop-title-hover = Suelta el archivo para abrirlo
drop-hint = o haz clic para elegir un archivo · todo se procesa en tu equipo

## Animación de apertura
phase-reading = Leyendo archivo…
phase-indexing = Indexando módulos…
phase-preparing = Preparando vista…
detail-indexing = Leyendo la estructura de la captura
detail-read-of = { $done } de { $total }
detail-read = { $size } leídos
detail-modules-found = { $count ->
    [one] { $count } módulo detectado
   *[other] { $count } módulos detectados
}

## Estado y pie
status-ready = Listo
status-decoding = Decodificando módulo…
status-decoding-inline = Decodificando…
status-saved = Guardado en { $path }
status-indexed = { $count ->
    [one] { $count } módulo indexado
   *[other] { $count } módulos indexados
}
status-copied = { $count ->
    [one] { $count } byte copiado al portapapeles
   *[other] { $count } bytes copiados al portapapeles
}
footer-build = v{ $version } · { $os } · { $arch }

## Etiquetas del workspace
label-modules = Módulos
label-line-numbers = Números de línea
label-no-capture = sin captura
label-compressed = { $size } comprimidos
label-counter = { $position } / { $total }
label-copy-suffix = · copia { $count }
label-find-counter = { $position } / { $total }
empty-selection = Elige un módulo en la lista para ver su contenido
empty-filter = Ningún módulo coincide con tu búsqueda

## Pistas de entrada
hint-filter = Filtrar módulos
hint-find = texto en este módulo

## Botones
button-open = Abrir…
button-copy = Copiar
button-save-as = Guardar como…
button-previous = Anterior
button-next = Siguiente
button-clear = Limpiar
button-toggle-rail = Mostrar u ocultar la lista de módulos
button-find = Buscar en este módulo

## Errores y estado del módulo
module-unreadable = Este módulo no se pudo indexar.
error-open = No se pudo abrir { $path }: { $reason }
error-module = No se pudo leer el módulo { $index }: { $reason }
error-save = No se pudo guardar: { $reason }

## Diálogos nativos
dialog-open-title = Abrir una captura de RouterOS
dialog-filter-name = Captura de RouterOS

## Actualizaciones
update-available = La versión { $version } está disponible
update-check = Buscar actualizaciones
update-now = Actualizar ahora
update-later = Más tarde
update-skip = Omitir esta versión
update-auto = Buscar actualizaciones automáticamente
