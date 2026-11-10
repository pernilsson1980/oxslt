<?xml version="1.0"?>
<!-- With the catalogs option, the catalogs in $SGML_CATALOG_FILES map the include's URL. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:include href="http://example.org/oxslt/inc.xsl"/>
  <xsl:output method="text"/>
  <xsl:template match="/"><xsl:value-of select="$from"/><xsl:text>&#10;</xsl:text></xsl:template>
</xsl:stylesheet>
