<?xml version="1.0"?>
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template name="load">
    <xsl:param name="uri"/>
    <xsl:value-of select="count(document($uri)/leaf)"/>
    <xsl:text>:</xsl:text>
    <xsl:value-of select="document($uri)/leaf"/>
  </xsl:template>
</xsl:stylesheet>
