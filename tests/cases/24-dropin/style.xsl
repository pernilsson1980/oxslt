<?xml version="1.0"?>
<!-- Details found by building Debian packages with oxslt (tests/dropin). -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:import href="sub/helper.xsl"/>
  <xsl:output method="text"/>
  <xsl:template match="/">
    <!-- A result tree fragment given to document() is a string, resolved
         against the instruction's module (sub/), as DocBook's man pages
         read their character map. -->
    <xsl:text>fragment uri: </xsl:text>
    <xsl:call-template name="load">
      <xsl:with-param name="uri"><xsl:value-of select="'deeper/leaf.xml'"/></xsl:with-param>
    </xsl:call-template>
    <xsl:text>&#10;string uri: </xsl:text>
    <xsl:call-template name="load">
      <xsl:with-param name="uri" select="'deeper/leaf.xml'"/>
    </xsl:call-template>
    <xsl:text>&#10;number with variables: </xsl:text>
    <xsl:for-each select="//n"><xsl:number level="any" count="n[$counted != 0][@c]" from="r[$from != 0]"/>,</xsl:for-each>
    <xsl:text>&#10;</xsl:text>
  </xsl:template>
  <xsl:variable name="counted" select="1"/>
  <xsl:variable name="from" select="1"/>
</xsl:stylesheet>
