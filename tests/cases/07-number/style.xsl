<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="text"/>
  <xsl:template match="sec">
    <xsl:number level="multiple" format="1.1.a"/><xsl:text> </xsl:text>
    <xsl:number level="any" format="i"/><xsl:text> </xsl:text>
    <xsl:number format="A"/><xsl:text> </xsl:text>
    <xsl:value-of select="t"/><xsl:text>&#10;</xsl:text>
    <xsl:apply-templates select="sec"/>
  </xsl:template>
  <xsl:template match="/"><xsl:apply-templates select="doc/sec"/><xsl:number value="1234567" grouping-separator="," grouping-size="3"/><xsl:text> </xsl:text><xsl:number value="28" format="01"/><xsl:text>&#10;</xsl:text></xsl:template>
</xsl:stylesheet>
