<?xml version="1.0"?>
<!-- A missing href is an error that doesn't stop the transform: later
     documents are written, but the main result is discarded (exit 9). -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:exsl="http://exslt.org/common" extension-element-prefixes="exsl">
  <xsl:template match="/">
    <main><exsl:document><never/></exsl:document><exsl:document href="later.xml"><written/></exsl:document></main>
  </xsl:template>
</xsl:stylesheet>
