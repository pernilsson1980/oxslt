<?xml version="1.0"?>
<!-- libxslt doesn't sort, or evaluate the sort keys of, fewer than two nodes; with two the undefined variable stops the transform (see 56). -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"><xsl:template match="/"><out><xsl:for-each select="*"><xsl:sort select="$nope"/>one</xsl:for-each></out></xsl:template>
</xsl:stylesheet>
