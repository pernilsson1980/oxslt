<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="text"/>
  <xsl:param name="greeting" select="'default'"/>
  <xsl:param name="factor" select="1"/>
  <xsl:param name="unset" select="'still default'"/>
  <xsl:variable name="v" select="number(/doc/v)"/>
  <xsl:template match="/">
    <xsl:value-of select="$greeting"/><xsl:text>|</xsl:text>
    <xsl:value-of select="$v * $factor"/><xsl:text>|</xsl:text>
    <xsl:value-of select="$unset"/><xsl:text>|</xsl:text>
    <xsl:call-template name="t"><xsl:with-param name="p" select="$v + 1"/></xsl:call-template>
    <xsl:text>&#10;</xsl:text>
  </xsl:template>
  <xsl:template name="t"><xsl:param name="p" select="0"/><xsl:param name="q" select="'q'"/><xsl:value-of select="concat($p,$q)"/></xsl:template>
</xsl:stylesheet>
