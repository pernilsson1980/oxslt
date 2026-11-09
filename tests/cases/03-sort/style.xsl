<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="text"/>
  <xsl:template match="/items">
    <xsl:text>numeric desc:</xsl:text>
    <xsl:for-each select="i"><xsl:sort select="@n" data-type="number" order="descending"/><xsl:text> </xsl:text><xsl:value-of select="@n"/></xsl:for-each>
    <xsl:text>&#10;text asc:</xsl:text>
    <xsl:for-each select="i"><xsl:sort select="@s"/><xsl:text> </xsl:text><xsl:value-of select="@s"/></xsl:for-each>
    <xsl:text>&#10;multi key:</xsl:text>
    <xsl:for-each select="i"><xsl:sort select="@n" data-type="number"/><xsl:sort select="@s" order="descending"/><xsl:text> </xsl:text><xsl:value-of select="concat(@n,':',@s)"/></xsl:for-each>
    <xsl:text>&#10;</xsl:text>
  </xsl:template>
</xsl:stylesheet>
