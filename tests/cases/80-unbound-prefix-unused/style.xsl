<!-- An unbound prefix on a function or variable is only an error when the
     expression is evaluated (libxml2 checks prefixes at compile time only in
     name tests): Boost's DocBook layout has stbl:convertLength in a branch
     that isn't taken. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:template match="/">
  <r>
    <xsl:if test="false()"><xsl:value-of select="stbl:convertLength(1)"/><xsl:value-of select="$p:v"/></xsl:if>
    <xsl:value-of select="function-available('concat')"/>
  </r>
</xsl:template>
</xsl:stylesheet>
