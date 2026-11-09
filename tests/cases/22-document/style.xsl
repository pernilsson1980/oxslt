<?xml version="1.0"?>
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="text"/>
  <xsl:strip-space elements="o"/>
  <xsl:key name="k" match="p" use="@id"/>
  <xsl:variable name="me" select="'x'"/>
  <xsl:template match="/">
    <!-- The input, by the URL it was given as: the same document. -->
    <xsl:text>same input: </xsl:text><xsl:value-of select="generate-id(document('in.xml')) = generate-id(/)"/>
    <xsl:text>&#10;other: </xsl:text><xsl:value-of select="document('other.xml')/o"/>
    <xsl:text>&#10;stripped: </xsl:text><xsl:value-of select="count(document('other.xml')/o/node())"/>
    <xsl:text>&#10;loaded once: </xsl:text><xsl:value-of select="generate-id(document('other.xml')) = generate-id(document('./other.xml'))"/>
    <xsl:text>&#10;by nodes: </xsl:text><xsl:for-each select="document(/r/ref)/*"><xsl:value-of select="name()"/>,</xsl:for-each>
    <xsl:text>&#10;node base: </xsl:text><xsl:value-of select="document(document('sub/inner.xml')/inner/next)"/>
    <xsl:text>&#10;xml:base: </xsl:text><xsl:value-of select="document(document('sub/inner.xml')/inner/nb)"/>
    <xsl:text>&#10;second arg: </xsl:text><xsl:value-of select="document('deeper/leaf.xml', document('sub/inner.xml'))"/>
    <xsl:text>&#10;missing: </xsl:text><xsl:value-of select="count(document('missing.xml'))"/>
    <xsl:text>&#10;fragment: </xsl:text><xsl:value-of select="document('other.xml#p2')"/>
    <xsl:text>&#10;self: </xsl:text><xsl:value-of select="document('')/*/xsl:variable/@select"/>
    <xsl:text>&#10;key: </xsl:text><xsl:for-each select="document('other.xml')"><xsl:value-of select="key('k', 'p1')"/></xsl:for-each>
    <xsl:text>&#10;id: </xsl:text><xsl:for-each select="document('other.xml')"><xsl:value-of select="count(id('p1 p2'))"/></xsl:for-each>
    <xsl:text>&#10;</xsl:text>
  </xsl:template>
</xsl:stylesheet>
