<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:variable name="who" select="'base'"/>
  <xsl:template match="item"><base><xsl:value-of select="."/></base></xsl:template>
  <xsl:template match="special"><base-special/></xsl:template>
</xsl:stylesheet>
