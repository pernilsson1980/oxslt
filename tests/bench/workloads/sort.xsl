<?xml version="1.0"?>
<!-- Sorting by a numeric and a text key. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="text"/>
  <xsl:template match="/records">
    <xsl:for-each select="r">
      <xsl:sort select="amount" data-type="number" order="descending"/>
      <xsl:sort select="name"/>
      <xsl:value-of select="concat(@id, ' ', name, ' ', amount, '&#10;')"/>
    </xsl:for-each>
  </xsl:template>
</xsl:stylesheet>
