<?xml version="1.0"?>
<!-- Lists every node of a document in document order, one entry per node:
       E {ns}local         element
       A {ns}local=[value] attribute
       T[text]             text
       C[text]             comment
       P target[data]      processing instruction
     Values are written raw between brackets. tests/parse_corpus.rs prints the
     same listing from oxslt's tree, so the two can be compared byte for byte. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="text" encoding="UTF-8"/>

  <xsl:template match="/">
    <xsl:apply-templates select="node()"/>
  </xsl:template>

  <xsl:template match="*">
    <xsl:text>E {</xsl:text><xsl:value-of select="namespace-uri()"/><xsl:text>}</xsl:text>
    <xsl:value-of select="local-name()"/><xsl:text>&#10;</xsl:text>
    <xsl:for-each select="@*">
      <xsl:text>A {</xsl:text><xsl:value-of select="namespace-uri()"/><xsl:text>}</xsl:text>
      <xsl:value-of select="local-name()"/><xsl:text>=[</xsl:text>
      <xsl:value-of select="."/><xsl:text>]&#10;</xsl:text>
    </xsl:for-each>
    <xsl:apply-templates select="node()"/>
  </xsl:template>

  <xsl:template match="text()">
    <xsl:text>T[</xsl:text><xsl:value-of select="."/><xsl:text>]&#10;</xsl:text>
  </xsl:template>

  <xsl:template match="comment()">
    <xsl:text>C[</xsl:text><xsl:value-of select="."/><xsl:text>]&#10;</xsl:text>
  </xsl:template>

  <xsl:template match="processing-instruction()">
    <xsl:text>P </xsl:text><xsl:value-of select="name()"/><xsl:text>[</xsl:text>
    <xsl:value-of select="."/><xsl:text>]&#10;</xsl:text>
  </xsl:template>
</xsl:stylesheet>
