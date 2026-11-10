<?xml version="1.0"?>
<!-- Globals of one name at the same import precedence (a stylesheet and
     its includes): the first in document order wins, as in libxslt, where
     xmlto's included encoding module beats a later included customisation.
     A param and a variable of one name aren't an error. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:import href="low.xsl"/>
  <xsl:include href="a.xsl"/>
  <xsl:include href="b.xsl"/>
  <xsl:output method="text"/>
  <xsl:param name="mixed" select="'main param'"/>
  <xsl:template match="/">
    <xsl:value-of select="concat($p, ' / ', $v, ' / ', $mixed, ' / ', $low)"/>
    <xsl:text>&#10;</xsl:text>
  </xsl:template>
</xsl:stylesheet>
