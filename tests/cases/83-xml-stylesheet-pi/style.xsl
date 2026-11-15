<?xml version="1.0"?>
<!-- Given as the stylesheet, a document with an <?xml-stylesheet?> before its
     root is transformed by the stylesheet that PI names (relative to the
     document), and the other arguments are ignored (emacspeak's build).
     A PI with another type, or after the root, doesn't count. -->
<?xml-stylesheet type="text/css" href="ignored.css"?>
<?xml-stylesheet href="real.xsl" type="text/xsl"?>
<doc><item>one</item><item>two</item></doc>
