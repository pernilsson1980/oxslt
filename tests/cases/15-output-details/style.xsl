<?xml version="1.0"?>
<!-- Output details found while matching libxml2's serializer (step 5.4):
     CDATA sections (an unprefixed name takes the default namespace; "]]>"
     splits a section), disable-output-escaping (kept when copied from a
     fragment), empty comments (written as nothing), indent="yes" (text
     children turn formatting off) and hex references for non-ASCII
     attribute values when xsl:output has no encoding. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
  xmlns="urn:out" exclude-result-prefixes="#default">
  <xsl:output indent="yes" cdata-section-elements="code"/>
  <xsl:variable name="raw"><xsl:text disable-output-escaping="yes">&lt;raw/&gt;</xsl:text></xsl:variable>
  <xsl:template match="/doc">
    <xsl:element name="root" namespace="urn:out">
      <xsl:attribute name="note">café ¥</xsl:attribute>
      <xsl:for-each select="item">
        <xsl:element name="code" namespace="urn:out"><xsl:value-of select="."/></xsl:element>
      </xsl:for-each>
      <xsl:element name="empty" namespace="urn:out"><xsl:comment/></xsl:element>
      <xsl:element name="nested" namespace="urn:out">
        <xsl:element name="leaf" namespace="urn:out"/>
        <xsl:element name="deeper" namespace="urn:out"><xsl:element name="leaf" namespace="urn:out"/></xsl:element>
      </xsl:element>
      <xsl:copy-of select="mixed"/>
      <xsl:element name="unescaped" namespace="urn:out"><xsl:value-of select="'&lt;b&gt;'" disable-output-escaping="yes"/></xsl:element>
      <xsl:element name="copied" namespace="urn:out"><xsl:copy-of select="$raw"/></xsl:element>
      <xsl:comment> after </xsl:comment>
    </xsl:element>
  </xsl:template>
</xsl:stylesheet>
