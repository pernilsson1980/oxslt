<?xml version="1.0"?>
<!-- xsl:attribute name="xmlns" is a compile error in libxslt (exit 5). -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"><xsl:template match="/"><out><xsl:attribute name="xmlns">v</xsl:attribute></out></xsl:template>
</xsl:stylesheet>
