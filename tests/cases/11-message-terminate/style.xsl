<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/"><start/><xsl:message>warning only</xsl:message><xsl:apply-templates/></xsl:template>
  <xsl:template match="bad"><xsl:message terminate="yes">fatal: bad element</xsl:message></xsl:template>
</xsl:stylesheet>
