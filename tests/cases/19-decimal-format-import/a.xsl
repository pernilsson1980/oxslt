<?xml version="1.0"?>
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:import href="a2.xsl"/>
  <xsl:output indent="yes" omit-xml-declaration="yes"/>
  <xsl:preserve-space elements="keep"/>
  <xsl:strip-space elements="sp"/>
  <xsl:param name="p" select="'a'"/>
  <xsl:param name="q" select="'a'"/>
  <xsl:decimal-format name="f" grouping-separator=" "/>
  <xsl:template match="a" priority="10"><a-a><xsl:apply-imports/></a-a></xsl:template>
  <xsl:template match="b"><a-b/></xsl:template>
  <xsl:template match="c"><a-c/></xsl:template>
  <xsl:template match="a" mode="m"><a-m/></xsl:template>
  <xsl:template name="named"><a-named/></xsl:template>
  <xsl:template name="only-a"><only-a/></xsl:template>
</xsl:stylesheet>
