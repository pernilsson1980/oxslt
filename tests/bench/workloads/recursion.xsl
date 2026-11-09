<?xml version="1.0"?>
<!-- Recursive named templates splitting strings word by word, as many
     real stylesheets do without EXSLT. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="text"/>
  <xsl:template match="/records">
    <xsl:for-each select="r">
      <xsl:call-template name="words"><xsl:with-param name="s" select="normalize-space(note)"/></xsl:call-template>
      <xsl:text>&#10;</xsl:text>
    </xsl:for-each>
  </xsl:template>
  <xsl:template name="words">
    <xsl:param name="s"/>
    <xsl:param name="n" select="1"/>
    <xsl:choose>
      <xsl:when test="contains($s, ' ')">
        <xsl:value-of select="concat($n, '=', string-length(substring-before($s, ' ')), ' ')"/>
        <xsl:call-template name="words">
          <xsl:with-param name="s" select="substring-after($s, ' ')"/>
          <xsl:with-param name="n" select="$n + 1"/>
        </xsl:call-template>
      </xsl:when>
      <xsl:otherwise><xsl:value-of select="concat($n, '=', string-length($s))"/></xsl:otherwise>
    </xsl:choose>
  </xsl:template>
</xsl:stylesheet>
