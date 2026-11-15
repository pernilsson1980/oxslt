<!-- A local variable may not redefine a variable or parameter of the same
     template (xsltRegisterVariable reports it, exit 9), except a parameter
     the caller passed: libxslt keeps that binding as the xsl:with-param,
     which may be redefined once (Boost's unparser.xsl does this). -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:output method="text"/>
<xsl:template match="/">
  <xsl:call-template name="t"><xsl:with-param name="p" select="'passed'"/></xsl:call-template>
  <xsl:call-template name="t"/>
  <xsl:call-template name="twice"><xsl:with-param name="p" select="'passed'"/></xsl:call-template>
</xsl:template>
<xsl:template name="t">
  <xsl:param name="p" select="'default'"/>
  <xsl:if test="true()"><xsl:variable name="p" select="concat($p, '+local')"/><xsl:value-of select="concat($p, '&#10;')"/></xsl:if>
</xsl:template>
<xsl:template name="twice">
  <xsl:param name="p"/>
  <xsl:variable name="p" select="'first'"/>
  <xsl:variable name="p" select="'second'"/>
  <xsl:value-of select="concat($p, '&#10;')"/>
</xsl:template>
</xsl:stylesheet>
