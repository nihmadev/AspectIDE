; Symbol-extraction tags for TypeScript / TSX (also drives JS/JSX). Each
; definition captures the identifier as `@name.<kind>` and the whole declaration
; as `@definition.<kind>` (its full extent, used for nesting/containment).

; ── Definitions ──
(function_declaration name: (identifier) @name.function) @definition.function
(generator_function_declaration name: (identifier) @name.function) @definition.function
(method_definition name: (property_identifier) @name.method) @definition.method
(class_declaration name: (type_identifier) @name.class) @definition.class
(abstract_class_declaration name: (type_identifier) @name.class) @definition.class
(interface_declaration name: (type_identifier) @name.interface) @definition.interface
(type_alias_declaration name: (type_identifier) @name.type) @definition.type
(enum_declaration name: (identifier) @name.enum) @definition.enum
(variable_declarator
  name: (identifier) @name.function
  value: [(arrow_function) (function_expression)]) @definition.function

; ── References ──
(call_expression function: (identifier) @reference.call)
(call_expression
  function: (member_expression property: (property_identifier) @reference.call))
(extends_clause value: (identifier) @reference.implement)
