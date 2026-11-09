<?xml version="1.0"?>
<!-- Grouping without keys: each record is compared with every earlier
     sibling, so the work is quadratic in the number of records. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="text"/>
  <xsl:template match="/records">
    <xsl:for-each select="r[not(@cat = preceding-sibling::r/@cat)]">
      <xsl:variable name="c" select="@cat"/>
      <xsl:value-of select="concat($c, ': ', count(../r[@cat = $c]), ' records, total ', sum(../r[@cat = $c]/amount), '&#10;')"/>
    </xsl:for-each>
  </xsl:template>
</xsl:stylesheet>
