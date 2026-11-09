<?xml version="1.0"?>
<!-- A variable in xsl:key's use is a compile error (libxslt: XML_XPATH_NOVAR). -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:variable name="v" select="'x'"/>
  <xsl:key name="k" match="a" use="concat(., $v)"/>
  <xsl:template match="/"><xsl:value-of select="count(key('k', '1x'))"/></xsl:template>
</xsl:stylesheet>
