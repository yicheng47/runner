; Highlights for tree-sitter-kotlin-ng, which ships none: written for Runner
; against its node types, under Runner's licence; see ../README.md. Later
; patterns take precedence.

(identifier) @variable

((identifier) @constant
  (#match? @constant "^[A-Z][A-Z0-9_]+$"))

((identifier) @constant.builtin
  (#any-of? @constant.builtin "true" "false" "null"))

(this_expression) @variable.special
(super_expression) @variable.special

(user_type (identifier) @type)
(type_alias type: (identifier) @type)
(class_declaration name: (identifier) @type)
(object_declaration name: (identifier) @type)
(enum_entry (identifier) @constant)

(class_parameter (identifier) @property)
(parameter (identifier) @variable.parameter)
(navigation_expression (identifier) @property .)

(function_declaration name: (identifier) @function)
(call_expression (identifier) @function)
(call_expression
  (navigation_expression (identifier) @function.method .))

(annotation) @attribute
(annotation (user_type (identifier) @attribute))
(annotation (constructor_invocation (user_type (identifier) @attribute)))

(package_header (qualified_identifier) @namespace)
(import (qualified_identifier) @namespace)

(line_comment) @comment
(block_comment) @comment
(shebang) @comment

(string_literal) @string
(multiline_string_literal) @string
(character_literal) @string
(escape_sequence) @string.escape
(interpolation ["$" "${" "}"] @punctuation.special)

(number_literal) @number
(float_literal) @number

(label) @label

[
  "abstract" "actual" "annotation" "as" "as?" "by" "catch" "class" "companion"
  "const" "constructor" "crossinline" "data" "do" "else" "enum" "expect"
  "external" "final" "finally" "for" "fun" "get" "if" "import" "in" "infix"
  "init" "inline" "inner" "interface" "internal" "is" "lateinit" "noinline"
  "object" "open" "operator" "out" "override" "package" "private" "protected"
  "public" "return" "sealed" "set" "suspend" "tailrec" "throw" "try"
  "typealias" "val" "value" "var" "vararg" "when" "where" "while"
] @keyword

[
  "!" "!=" "!==" "%" "%=" "&&" "*" "*=" "+" "++" "+=" "-" "--" "-=" "->"
  ".." "..<" "/" "/=" "<" "<=" "=" "==" "===" ">" ">=" "?:" "||" "!!" "::"
] @operator

["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["." "?." "," ";" ":"] @punctuation.delimiter
