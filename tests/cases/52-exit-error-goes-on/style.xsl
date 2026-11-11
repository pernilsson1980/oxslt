<?xml version="1.0"?>
<!-- Errors libxslt only reports leave it in its error state and go on: exit 9, no output. Here an attribute after children, a redefined variable, an undefined template, and xsl:for-each over a string. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"><xsl:template match="/"><out><e><c/><xsl:attribute name="a">v</xsl:attribute></e><xsl:variable name="v" select="1"/><xsl:variable name="v" select="2"/><xsl:call-template name="missing"/><xsl:for-each select="1">x</xsl:for-each><xsl:message>still running</xsl:message></out></xsl:template>
</xsl:stylesheet>
