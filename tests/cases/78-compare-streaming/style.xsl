<!-- `x = path` stops at the first matching node when the path is pure. A
     predicate calling generate-id() isn't pure: libxml2 builds the whole
     node-set, so every node it passes gets its id, and later ids depend on
     that. Grouping without keys, with and without it. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:output method="text"/>
<xsl:template match="/doc">
  <xsl:text>groups:</xsl:text>
  <xsl:for-each select="r[not(@k = preceding-sibling::r/@k)]"><xsl:value-of select="concat(' ', @k)"/></xsl:for-each>
  <xsl:text>&#10;numbers:</xsl:text>
  <xsl:for-each select="r[not(@k = preceding-sibling::r[@n &gt; 0]/@k)]"><xsl:value-of select="concat(' ', @n)"/></xsl:for-each>
  <xsl:text>&#10;with ids:</xsl:text>
  <xsl:for-each select="r[3][@k = preceding-sibling::r[generate-id() != '']/@k]"><xsl:value-of select="concat(' ', @n)"/></xsl:for-each>
  <xsl:text>&#10;ids:</xsl:text>
  <xsl:for-each select="r"><xsl:value-of select="concat(' ', generate-id())"/></xsl:for-each>
  <xsl:text>&#10;</xsl:text>
</xsl:template>
</xsl:stylesheet>
