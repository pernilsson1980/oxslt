<?xml version="1.0"?>
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:param name="q" select="'b'"/>
  <!-- B is a sibling of A: does apply-imports here reach A's rules? -->
  <xsl:template match="b"><b-b><xsl:apply-imports/></b-b></xsl:template>
  <xsl:template match="a" priority="20"><b-a><xsl:apply-imports/></b-a></xsl:template>
</xsl:stylesheet>
