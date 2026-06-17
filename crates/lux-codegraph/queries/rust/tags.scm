; Symbol-extraction tags for Rust.
; Each definition pattern captures TWO nodes: the identifier as `@name.<kind>`
; (the symbol's name + navigation span) and the whole item as
; `@definition.<kind>` (its full lexical extent, used for nesting/containment).
; References capture a single `@reference.<kind>` identifier.

; ── Definitions ──
(function_item name: (identifier) @name.function) @definition.function
(function_signature_item name: (identifier) @name.function) @definition.function
(struct_item name: (type_identifier) @name.struct) @definition.struct
(union_item name: (type_identifier) @name.struct) @definition.struct
(enum_item name: (type_identifier) @name.enum) @definition.enum
(trait_item name: (type_identifier) @name.interface) @definition.interface
(type_item name: (type_identifier) @name.type) @definition.type
(const_item name: (identifier) @name.constant) @definition.constant
(static_item name: (identifier) @name.constant) @definition.constant
(mod_item name: (identifier) @name.module) @definition.module
(macro_definition name: (identifier) @name.macro) @definition.macro

; ── References ──
(call_expression function: (identifier) @reference.call)
(call_expression function: (field_expression field: (field_identifier) @reference.call))
(call_expression function: (scoped_identifier name: (identifier) @reference.call))
(macro_invocation macro: (identifier) @reference.call)
(impl_item trait: (type_identifier) @reference.implement)
