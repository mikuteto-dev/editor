((script_element
  (start_tag) @_tag
  (raw_text) @injection.content)
 (#not-match? @_tag "\\blang\\s*=")
 (#set! injection.language "javascript"))

((style_element
  (start_tag) @_tag
  (raw_text) @injection.content)
 (#not-match? @_tag "\\blang\\s*=")
 (#set! injection.language "css"))

([ (script_element
     (start_tag
       (attribute
         (attribute_name) @_attr
         [ (attribute_value) @injection.language
           (quoted_attribute_value (attribute_value) @injection.language) ]))
     (raw_text) @injection.content)
   (style_element
     (start_tag
       (attribute
         (attribute_name) @_attr
         [ (attribute_value) @injection.language
           (quoted_attribute_value (attribute_value) @injection.language) ]))
     (raw_text) @injection.content) ]
 (#eq? @_attr "lang"))

((interpolation (raw_text) @injection.content)
 (#set! injection.language "typescript"))

((directive_attribute
  [ (attribute_value) @injection.content
    (quoted_attribute_value (attribute_value) @injection.content) ])
 (#set! injection.language "typescript"))
