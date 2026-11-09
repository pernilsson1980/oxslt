<?xml version="1.0"?>
<!-- Namespace and attribute-set rules found while matching libxslt (step 5.4):
     - a literal result element directly in a template body inherits the
       template's namespaces; one inside xsl:if doesn't
     - xsl:exclude-result-prefixes on a literal element doesn't affect its own
       declarations
     - xsl:namespace-alias keeps the stylesheet's prefix
     - xsl:attribute without a prefix gets ns_1 at the top; a prefix bound
       differently above is redeclared
     - attribute sets: own attributes first, then used sets'; caller locals
       are visible
     - strip-space/preserve-space: for the same name the first one wins -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
  xmlns:keep="urn:keep" xmlns:alias="urn:alias" xmlns:real="urn:real"
  xmlns:n1="urn:n1" xmlns:n2="urn:n2" exclude-result-prefixes="n1 n2">
  <xsl:namespace-alias stylesheet-prefix="alias" result-prefix="real"/>
  <xsl:strip-space elements="n1:t n2:t"/>
  <xsl:preserve-space elements="n1:t"/>
  <xsl:attribute-set name="base"><xsl:attribute name="b">base</xsl:attribute><xsl:attribute name="shared">base</xsl:attribute></xsl:attribute-set>
  <xsl:attribute-set name="top" use-attribute-sets="base"><xsl:attribute name="t">top</xsl:attribute><xsl:attribute name="shared">top</xsl:attribute><xsl:attribute name="v"><xsl:value-of select="$v"/></xsl:attribute></xsl:attribute-set>
  <xsl:variable name="v" select="'global'"/>
  <xsl:template match="/doc">
    <xsl:variable name="v" select="'local'"/>
    <out xmlns:own="urn:own" xsl:exclude-result-prefixes="own" xsl:use-attribute-sets="top">
      <xsl:attribute name="plain" namespace="urn:keep">1</xsl:attribute>
      <xsl:if test="true()"><nested/></xsl:if>
      <alias:aliased alias:attr="x"/>
      <xsl:element name="keep:redeclared"><xsl:attribute name="keep:p" namespace="urn:other">2</xsl:attribute></xsl:element>
      <stripped><xsl:copy-of select="*"/></stripped>
    </out>
  </xsl:template>
</xsl:stylesheet>
