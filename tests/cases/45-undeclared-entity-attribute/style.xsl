<?xml version="1.0"?>
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="text"/>
  <xsl:template match="/">
    <xsl:for-each select="//@*">
      <xsl:value-of select="concat(name(), '=[', ., ']&#10;')"/>
    </xsl:for-each>
    <xsl:value-of select="concat('text=', string(a/b), ' nodes=', count(a/b/text()), '&#10;')"/>
  </xsl:template>
</xsl:stylesheet>
