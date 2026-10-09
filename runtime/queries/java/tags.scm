(class_declaration
  name: (identifier) @name) @definition.class

(interface_declaration
  name: (identifier) @name) @definition.interface

(record_declaration
  name: (identifier) @name) @definition.class

(enum_declaration
  name: (identifier) @name) @definition.class

(method_declaration
  name: (identifier) @name) @definition.function

(constructor_declaration
  name: (identifier) @name) @definition.function

(compact_constructor_declaration
  name: (identifier) @name) @definition.function

(field_declaration
  declarator: (variable_declarator
    name: (identifier) @name)) @definition.constant

(enum_constant
  name: (identifier) @name) @definition.constant
