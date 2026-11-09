<?xml version="1.0"?>
<!-- dyn:evaluate, as libexslt implements it. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:dyn="http://exslt.org/dynamic" xmlns:q="urn:p" extension-element-prefixes="dyn">
  <xsl:output method="text"/>
  <xsl:key name="bycat" match="item" use="@cat"/>
  <xsl:variable name="n" select="2"/>
  <xsl:template match="/">
    <xsl:text>available: </xsl:text><xsl:value-of select="function-available('dyn:evaluate')"/>
    <xsl:text>&#10;arithmetic: </xsl:text><xsl:value-of select="dyn:evaluate('1 + 2 * 3')"/>
    <xsl:text>&#10;path: </xsl:text><xsl:value-of select="dyn:evaluate('r/item[2]')"/>
    <xsl:text>&#10;count: </xsl:text><xsl:value-of select="count(dyn:evaluate('//item'))"/>
    <xsl:text>&#10;variable: </xsl:text><xsl:value-of select="dyn:evaluate('r/item[$n]/@n')"/>
    <xsl:text>&#10;prefix: </xsl:text><xsl:value-of select="dyn:evaluate('r/q:thing')"/>
    <xsl:text>&#10;new name: </xsl:text><xsl:value-of select="count(dyn:evaluate('//never-seen-before'))"/>
    <xsl:text>&#10;key: </xsl:text><xsl:value-of select="count(dyn:evaluate(&quot;key('bycat', 'a')&quot;))"/>
    <xsl:text>&#10;document: </xsl:text><xsl:value-of select="dyn:evaluate(&quot;document('other.xml')/other/v&quot;)"/>
    <xsl:text>&#10;empty: </xsl:text><xsl:value-of select="count(dyn:evaluate(''))"/>
    <xsl:text>&#10;syntax error: </xsl:text><xsl:value-of select="count(dyn:evaluate('1 +'))"/>
    <xsl:text>&#10;predicate: </xsl:text>
    <xsl:for-each select="r/item[dyn:evaluate('position() &gt; 1')]"><xsl:value-of select="."/>,</xsl:for-each>
    <xsl:text>&#10;by string: </xsl:text>
    <xsl:variable name="expr">@cat = 'a'</xsl:variable>
    <xsl:for-each select="r/item[dyn:evaluate($expr)]"><xsl:value-of select="@n"/>,</xsl:for-each>
    <xsl:text>&#10;current: </xsl:text>
    <xsl:for-each select="r/item"><xsl:value-of select="dyn:evaluate('current()/@n * 10')"/>,</xsl:for-each>
    <xsl:text>&#10;</xsl:text>
  </xsl:template>
</xsl:stylesheet>
