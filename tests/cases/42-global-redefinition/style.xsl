<?xml version="1.0"?>
<!-- Two global xsl:variables of one name in a stylesheet and its includes
     are a compile error in libxslt ("redefinition of global variable"). -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:include href="inc.xsl"/>
  <xsl:output method="text"/>
  <xsl:variable name="v" select="'main'"/>
  <xsl:template match="/"><xsl:value-of select="$v"/></xsl:template>
</xsl:stylesheet>
