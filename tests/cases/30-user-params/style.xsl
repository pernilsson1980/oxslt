<?xml version="1.0"?>
<!-- Command-line parameters as libxslt treats them: undeclared ones are
     globals too, and an xsl:variable of the same name wins. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="text"/>
  <xsl:variable name="shadowed" select="'stylesheet variable'"/>
  <xsl:param name="declared" select="'default'"/>
  <xsl:template match="/">
    <xsl:text>undeclared: </xsl:text><xsl:value-of select="$undeclared"/>
    <xsl:text>&#10;expr: </xsl:text><xsl:value-of select="$expr"/>
    <xsl:text>&#10;shadowed: </xsl:text><xsl:value-of select="$shadowed"/>
    <xsl:text>&#10;declared: </xsl:text><xsl:value-of select="$declared"/>
    <xsl:text>&#10;</xsl:text>
  </xsl:template>
</xsl:stylesheet>
