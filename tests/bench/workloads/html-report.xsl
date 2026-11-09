<?xml version="1.0"?>
<!-- A typical report: templates, attribute value templates, conditions and
     number formatting into an HTML table. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="html"/>
  <xsl:template match="/records">
    <html><head><title>Report</title></head><body>
      <h1>Records: <xsl:value-of select="count(r)"/></h1>
      <table><xsl:apply-templates select="r"/></table>
    </body></html>
  </xsl:template>
  <xsl:template match="r">
    <tr class="{@cat}" id="row-{@id}">
      <td><xsl:value-of select="name"/></td>
      <td><xsl:value-of select="format-number(amount, '#,##0.00')"/></td>
      <td><xsl:choose>
        <xsl:when test="amount &gt; 500">high</xsl:when>
        <xsl:when test="amount &gt; 100">medium</xsl:when>
        <xsl:otherwise>low</xsl:otherwise>
      </xsl:choose></td>
      <td><xsl:value-of select="translate(note, 'abcdefghijklmnopqrstuvwxyz', 'ABCDEFGHIJKLMNOPQRSTUVWXYZ')"/></td>
    </tr>
  </xsl:template>
</xsl:stylesheet>
