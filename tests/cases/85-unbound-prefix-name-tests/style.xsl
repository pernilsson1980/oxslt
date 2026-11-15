<!-- In an expression, an unbound prefix in a name test is an error only when
     the step is evaluated (libxslt compiles expressions without
     XML_XPATH_CHECKNS): the key's, the xsl:if's and the unused template's
     never are. But a step is evaluated even with no context nodes:
     libxml2 checks the prefix first, so doc[false()]/p:x stops the
     transform (exit 10). Patterns reject an unbound prefix at once. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:key name="k" match="*" use="p:x"/>
<xsl:template match="/"><r a="{count(doc[false()]/p:x)}"><xsl:if test="false() and p:*">no</xsl:if>done</r></xsl:template>
<xsl:template name="unused"><xsl:value-of select="p:x"/><xsl:for-each select="q:*"/><x b="{p:y}"/></xsl:template>
</xsl:stylesheet>
