<?xml version="1.0"?>
<!-- Lists an HTML document's nodes with their depth, for comparing the
     trees oxslt's and xsltproc's the html option of parsers build:
       E depth name line          element (saxon:line-number)
       A name=[value]             attribute
       I id                       an ID that id() finds this element by
       T[text]  C[text]  P target[data]
-->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:saxon="http://icl.com/saxon" exclude-result-prefixes="saxon">
  <xsl:output method="text" encoding="UTF-8"/>

  <xsl:template match="/">
    <xsl:apply-templates select="node()"/>
  </xsl:template>

  <xsl:template match="*">
    <xsl:variable name="me" select="generate-id()"/>
    <xsl:value-of select="concat('E ', count(ancestor::*), ' ', name(), ' ', saxon:line-number(), '&#10;')"/>
    <xsl:for-each select="@*">
      <xsl:value-of select="concat('A ', name(), '=[', ., ']&#10;')"/>
      <xsl:if test="generate-id(id(string(.))) = $me">
        <xsl:value-of select="concat('I ', ., '&#10;')"/>
      </xsl:if>
    </xsl:for-each>
    <xsl:apply-templates select="node()"/>
  </xsl:template>

  <xsl:template match="text()">
    <xsl:value-of select="concat('T[', ., ']&#10;')"/>
  </xsl:template>

  <xsl:template match="comment()">
    <xsl:value-of select="concat('C[', ., ']&#10;')"/>
  </xsl:template>

  <xsl:template match="processing-instruction()">
    <xsl:value-of select="concat('P ', name(), '[', ., ']&#10;')"/>
  </xsl:template>
</xsl:stylesheet>
