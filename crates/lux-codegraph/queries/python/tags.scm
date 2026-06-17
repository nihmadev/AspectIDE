; Symbol-extraction tags for Python. Each definition captures the identifier as
; `@name.<kind>` and the whole def/class as `@definition.<kind>` (its full extent,
; used for nesting/containment). References capture a single identifier.

; ── Definitions ──
(function_definition name: (identifier) @name.function) @definition.function
(class_definition name: (identifier) @name.class) @definition.class

; ── References ──
(call function: (identifier) @reference.call)
(call function: (attribute attribute: (identifier) @reference.call))
(import_statement name: (dotted_name (identifier) @reference.import))
(import_from_statement name: (dotted_name (identifier) @reference.import))
