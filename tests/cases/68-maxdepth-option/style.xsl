<!-- libxslt counts nested sequence constructors toward maxdepth, not
     template calls: here each level is a template body and an xsl:if, so
     1,600 levels are 3,200 deep. With maxdepth 4000 it finishes. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:template match="/"><r><xsl:call-template name="down"><xsl:with-param name="n" select="1600"/></xsl:call-template></r></xsl:template>
<xsl:template name="down">
  <xsl:param name="n"/>
  <xsl:if test="$n &gt; 0"><xsl:call-template name="down"><xsl:with-param name="n" select="$n - 1"/></xsl:call-template></xsl:if>
  <xsl:if test="$n = 0">bottom</xsl:if>
</xsl:template>
</xsl:stylesheet>
