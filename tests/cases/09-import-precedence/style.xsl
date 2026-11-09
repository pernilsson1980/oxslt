<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:import href="base.xsl"/>
  <xsl:include href="inc.xsl"/>
  <xsl:output method="xml" omit-xml-declaration="yes"/>
  <xsl:variable name="who" select="'main'"/>
  <xsl:template match="/r"><out who="{$who}"><xsl:apply-templates/><xsl:call-template name="from-include"/></out></xsl:template>
  <xsl:template match="item"><main><xsl:apply-imports/></main></xsl:template>
</xsl:stylesheet>
