<?xml version="1.0"?>
<!-- libxml2's XPath compiler skips a '/' at the start of a relative
     location path, and takes '//' there as another descendant-or-self step:
     ///b is //b, $r///b is $r//b. A lone '/' not followed by a step is
     skipped, so / /a is /a. (a///b is an error.) -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="text"/>
  <xsl:template match="/">
    <xsl:variable name="r" select="/"/>
    <xsl:value-of select="count(///b)"/><xsl:text> </xsl:text>
    <xsl:value-of select="count(////b)"/><xsl:text> </xsl:text>
    <xsl:value-of select="count($r///b)"/><xsl:text> </xsl:text>
    <xsl:value-of select="count($r////b)"/><xsl:text> </xsl:text>
    <xsl:value-of select="count(/ /a)"/><xsl:text> </xsl:text>
    <xsl:value-of select="count(/ //b)"/><xsl:text> </xsl:text>
    <xsl:value-of select="count(///signal) &gt; 0"/><xsl:text> </xsl:text>
    <xsl:for-each select="///c"><xsl:value-of select="name()"/></xsl:for-each>
    <xsl:text>&#10;</xsl:text>
  </xsl:template>
</xsl:stylesheet>
