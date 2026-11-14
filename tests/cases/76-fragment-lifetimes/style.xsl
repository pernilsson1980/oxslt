<!-- Result tree fragments of a template call's variables are freed when it
     returns; these are the cases where a fragment must outlive that: a
     global first computed inside a template, a fragment passed down as a
     parameter, exsl:node-set() of a local fragment while other templates
     run, and generate-id() on fragments made after earlier ones were
     freed (no number may be reused). -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:exsl="http://exslt.org/common" exclude-result-prefixes="exsl">
<xsl:output indent="yes"/>
<xsl:variable name="g"><item>global-1</item><item>global-2</item></xsl:variable>
<xsl:variable name="gset" select="exsl:node-set($g)/item"/>

<xsl:template match="/">
  <out>
    <xsl:for-each select="doc/x">
      <xsl:call-template name="outer"><xsl:with-param name="n" select="."/></xsl:call-template>
    </xsl:for-each>
    <after-calls global="{count($gset)}" first="{$gset[1]}"/>
    <ids>
      <xsl:for-each select="doc/x">
        <xsl:call-template name="id"/>
      </xsl:for-each>
    </ids>
  </out>
</xsl:template>

<xsl:template name="outer">
  <xsl:param name="n"/>
  <xsl:variable name="local"><v><xsl:value-of select="$n"/></v><v>second</v></xsl:variable>
  <!-- The global is first computed here, inside a call. -->
  <call n="{$n}" global="{$gset[2]}">
    <xsl:for-each select="exsl:node-set($local)/v">
      <xsl:call-template name="inner"><xsl:with-param name="frag" select="$local"/></xsl:call-template>
    </xsl:for-each>
  </call>
</xsl:template>

<xsl:template name="inner">
  <xsl:param name="frag"/>
  <xsl:variable name="tmp"><t>inner</t></xsl:variable>
  <inner value="{.}" passed="{count(exsl:node-set($frag)/v)}" tmp="{$tmp}"/>
</xsl:template>

<xsl:template name="id">
  <xsl:variable name="f"><a/><b/></xsl:variable>
  <id a="{generate-id(exsl:node-set($f)/a)}" b="{generate-id(exsl:node-set($f)/b)}" x="{generate-id(.)}"/>
</xsl:template>
</xsl:stylesheet>
