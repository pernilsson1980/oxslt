<?xml version="1.0"?>
<!-- Imports: A then B, so B has higher precedence than A; A imports A2. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:import href="a.xsl"/>
  <xsl:import href="b.xsl"/>
  <xsl:output indent="no"/>
  <xsl:strip-space elements="keep"/>
  <xsl:param name="p" select="'main'"/>

  <xsl:template match="/">
    <out p="{$p}" q="{$q}" n="{format-number(r/n, '#.##0,0', 'f')}">
      <xsl:apply-templates select="r/*"/>
      <xsl:apply-templates select="r/a" mode="m"/>
      <xsl:call-template name="named"/>
      <xsl:call-template name="only-a"/>
    </out>
  </xsl:template>

  <!-- Lower priority than A's and B's rules, but higher precedence. -->
  <xsl:template match="a" priority="-5"><main-a><xsl:apply-imports/></main-a></xsl:template>
  <xsl:template match="a" mode="m"><main-m><xsl:apply-imports/></main-m></xsl:template>
  <xsl:template name="named"><main-named><xsl:apply-imports/></main-named></xsl:template>
  <xsl:template match="sp|keep">[<xsl:value-of select="name()"/>:<xsl:value-of select="count(node())"/>]</xsl:template>
</xsl:stylesheet>
