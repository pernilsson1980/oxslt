<?xml version="1.0"?>
<!-- XInclude as xsltproc processes it (run with the xinclude option). -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="xml" indent="no"/>
  <xsl:template match="/">
    <report>
      <xsl:text>&#10;</xsl:text>
      <xsl:copy-of select="/book"/>
      <xsl:text>&#10;text nodes in #t: </xsl:text><xsl:value-of select="count(id('t')/text())"/>
      <xsl:text>&#10;bases: </xsl:text>
      <xsl:for-each select="//@xml:base"><xsl:value-of select="name(..)"/>=<xsl:value-of select="."/>,</xsl:for-each>
      <xsl:text>&#10;ids: </xsl:text><xsl:value-of select="count(id('c1'))"/>,<xsl:value-of select="count(id('second'))"/>
      <xsl:text>&#10;</xsl:text>
    </report>
  </xsl:template>
</xsl:stylesheet>
