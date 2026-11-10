<?xml version="1.0"?>
<!-- With nowrite, exsl:document writes nothing and its body doesn't run. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:exsl="http://exslt.org/common" extension-element-prefixes="exsl">
  <xsl:template match="/">
    <main><exsl:document href="refused.xml"><x/><xsl:message>not shown</xsl:message></exsl:document><after/></main>
  </xsl:template>
</xsl:stylesheet>
