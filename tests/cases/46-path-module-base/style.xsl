<?xml version="1.0"?>
<!-- lib.xsl isn't next to this stylesheet; xsltproc finds it in the path option
     directory, and names it by the path found, so document() calls in it
     resolve from lib/ (DocBook's l10n files, found this way by opensc). -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:import href="lib.xsl"/>
  <xsl:output method="text"/>
  <xsl:template match="/">
    <xsl:value-of select="concat($data, ' / ', $from-include, '&#10;')"/>
  </xsl:template>
</xsl:stylesheet>
