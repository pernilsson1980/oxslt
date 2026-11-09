<?xml version="1.0"?>
<!-- libxslt never pops a module's exclude-result-prefixes: an included
     module's exclusions also apply to the including module's later
     templates (yelp-xsl's Mallard cache depends on it). An imported module
     starts afresh. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:a="urn:a" xmlns:b="urn:b">
  <xsl:import href="imported.xsl"/>
  <xsl:output omit-xml-declaration="yes"/>
  <xsl:template match="r" mode="before"><before/></xsl:template>
  <xsl:include href="included.xsl"/>
  <xsl:template match="/">
    <out><xsl:apply-templates select="r" mode="before"/><xsl:apply-templates select="r" mode="after"/><xsl:apply-templates select="r" mode="imported"/></out>
  </xsl:template>
  <xsl:template match="r" mode="after"><after/></xsl:template>
</xsl:stylesheet>
