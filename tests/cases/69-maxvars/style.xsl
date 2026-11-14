<!-- libxslt stops a template call when 15,000 variables and parameters
     (all calls' together) are on its stack: six parameters a call, 2,600
     calls deep, is under maxdepth 9000 but over the variable limit. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:template match="/"><r><xsl:call-template name="down"><xsl:with-param name="n" select="2600"/></xsl:call-template></r></xsl:template>
<xsl:template name="down">
  <xsl:param name="n"/><xsl:param name="a" select="1"/><xsl:param name="b" select="2"/>
  <xsl:param name="c" select="3"/><xsl:param name="d" select="4"/><xsl:param name="e" select="5"/>
  <xsl:if test="$n &gt; 0"><xsl:call-template name="down"><xsl:with-param name="n" select="$n - 1"/></xsl:call-template></xsl:if>
  <xsl:if test="$n = 0">bottom</xsl:if>
</xsl:template>
</xsl:stylesheet>
