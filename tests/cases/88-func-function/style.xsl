<!-- EXSLT func:function and func:result (libexslt's functions module), as
     Erlang's documentation stylesheets use them: arguments and defaults,
     recursion, results from select or from content (a fragment), node-set
     results, the body's context node (the instruction's current node, even
     inside a predicate), globals, function-available(), and an imported
     definition overridden by the importing stylesheet's. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:func="http://exslt.org/functions" xmlns:my="urn:my"
    xmlns:exsl="http://exslt.org/common"
    extension-element-prefixes="func" exclude-result-prefixes="my exsl">
<xsl:import href="imported.xsl"/>
<xsl:output method="text"/>
<xsl:variable name="suffix" select="'!'"/>

<func:function name="my:greet">
  <xsl:param name="who"/>
  <xsl:param name="greeting" select="'Hello'"/>
  <func:result select="concat($greeting, ', ', $who, $suffix)"/>
</func:function>

<func:function name="my:fact">
  <xsl:param name="n"/>
  <xsl:choose>
    <xsl:when test="$n &lt;= 1"><func:result select="1"/></xsl:when>
    <xsl:otherwise><func:result select="$n * my:fact($n - 1)"/></xsl:otherwise>
  </xsl:choose>
</func:function>

<func:function name="my:wrap">
  <xsl:param name="s"/>
  <func:result><w><xsl:value-of select="$s"/></w><w>second</w></func:result>
</func:function>

<func:function name="my:items">
  <func:result select="//item[@keep]"/>
</func:function>

<func:function name="my:here">
  <func:result select="name(.)"/>
</func:function>

<func:function name="my:nothing">
  <xsl:variable name="unused" select="1"/>
</func:function>

<func:function name="my:which">
  <func:result select="'importer'"/>
</func:function>

<xsl:template match="/">
  <xsl:value-of select="my:greet('world')"/><xsl:text>&#10;</xsl:text>
  <xsl:value-of select="my:greet('you', 'Hi')"/><xsl:text>&#10;</xsl:text>
  <xsl:value-of select="my:fact(6)"/><xsl:text>&#10;</xsl:text>
  <xsl:value-of select="count(exsl:node-set(my:wrap('x'))/w)"/>:<xsl:value-of select="my:wrap('x')"/><xsl:text>&#10;</xsl:text>
  <xsl:value-of select="count(my:items())"/>/<xsl:value-of select="my:items()[2]"/><xsl:text>&#10;</xsl:text>
  <xsl:for-each select="doc"><xsl:value-of select="count(item[my:here() = 'doc'])"/></xsl:for-each><xsl:text>&#10;</xsl:text>
  [<xsl:value-of select="my:nothing()"/>]<xsl:text>&#10;</xsl:text>
  <xsl:value-of select="my:which()"/>,<xsl:value-of select="my:only-imported()"/><xsl:text>&#10;</xsl:text>
  <xsl:value-of select="function-available('my:greet')"/>,<xsl:value-of select="function-available('my:missing')"/>,<xsl:value-of select="element-available('func:result')"/><xsl:text>&#10;</xsl:text>
</xsl:template>
</xsl:stylesheet>
