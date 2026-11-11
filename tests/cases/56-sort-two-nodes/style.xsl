<?xml version="1.0"?>
<!-- A sort key that fails to evaluate stops the transform: exit 10. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"><xsl:template match="/"><out><xsl:for-each select="*/*"><xsl:sort select="$nope"/>x</xsl:for-each></out></xsl:template>
</xsl:stylesheet>
