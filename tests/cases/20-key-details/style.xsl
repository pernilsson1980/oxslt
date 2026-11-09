<?xml version="1.0"?>
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:p="urn:p" xmlns:q="urn:p">
  <xsl:output method="text"/>
  <xsl:key name="by-tag" match="item" use="tag"/>
  <xsl:key name="by-cat" match="item" use="@cat"/>
  <!-- Two definitions with one name add up. -->
  <xsl:key name="code" match="item" use="@cat"/>
  <xsl:key name="code" match="other" use="@code"/>
  <xsl:key name="p:pre" match="item" use="@id"/>
  <xsl:key name="attr" match="@cat" use="."/>
  <xsl:key name="text" match="text()" use="."/>
  <xsl:key name="cur" match="item" use="generate-id(current())"/>

  <xsl:template match="/">
    <xsl:text>tag y: </xsl:text><xsl:for-each select="key('by-tag', 'y')"><xsl:value-of select="@id"/>,</xsl:for-each>
    <xsl:text>&#10;tags of i1: </xsl:text><xsl:for-each select="key('by-tag', //item[1]/tag)"><xsl:value-of select="@id"/>,</xsl:for-each>
    <xsl:text>&#10;cat 'a b': </xsl:text><xsl:value-of select="count(key('by-cat', 'a b'))"/>
    <xsl:text>&#10;cat 'a': </xsl:text><xsl:value-of select="count(key('by-cat', 'a'))"/>
    <xsl:text>&#10;code a: </xsl:text><xsl:for-each select="key('code', 'a')"><xsl:value-of select="name()"/>,</xsl:for-each>
    <xsl:text>&#10;code a b: </xsl:text><xsl:for-each select="key('code', 'a b')"><xsl:value-of select="name()"/>,</xsl:for-each>
    <xsl:text>&#10;prefixed: </xsl:text><xsl:value-of select="key('q:pre', 'i2')/sub"/>
    <xsl:text>&#10;undeclared: </xsl:text><xsl:value-of select="count(key('nope', 'i2'))"/>
    <xsl:text>&#10;attr: </xsl:text><xsl:value-of select="name(key('attr', 'c'))"/>
    <xsl:text>&#10;text: </xsl:text><xsl:value-of select="count(key('text', 'y'))"/>
    <xsl:text>&#10;number: </xsl:text><xsl:value-of select="count(key('by-tag', 1))"/>
    <xsl:text>&#10;cur: </xsl:text><xsl:value-of select="key('cur', generate-id(//item[3]))/@id"/>
    <xsl:text>&#10;</xsl:text>
    <xsl:apply-templates select="//sub"/>
  </xsl:template>

  <xsl:template match="key('by-tag', 'z')/sub">key/: <xsl:value-of select="."/>&#10;</xsl:template>
  <xsl:template match="key('by-cat', 'b')//sub">key//: <xsl:value-of select="."/>&#10;</xsl:template>
  <xsl:template match="id('i1')//sub">id//: <xsl:value-of select="."/>&#10;</xsl:template>
  <!-- Two predicates make libxslt match "directly", as the XPath
       "//key(…)…", which is invalid: this never matches. -->
  <xsl:template match="key('by-cat','c')//sub[1][.='s3']" priority="9">direct: <xsl:value-of select="."/>&#10;</xsl:template>
  <xsl:template match="sub">plain: <xsl:value-of select="."/>&#10;</xsl:template>
</xsl:stylesheet>
