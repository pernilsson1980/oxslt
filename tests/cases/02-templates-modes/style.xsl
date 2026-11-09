<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="xml" indent="yes"/>
  <xsl:template match="/book">
    <out><toc><xsl:apply-templates select="chapter" mode="toc"/></toc><xsl:apply-templates/></out>
  </xsl:template>
  <xsl:template match="chapter" mode="toc"><entry n="{@n}"><xsl:value-of select="title"/></entry></xsl:template>
  <xsl:template match="chapter"><section><xsl:apply-templates select="para"/></section></xsl:template>
  <xsl:template match="para"><p><xsl:value-of select="."/></p></xsl:template>
  <xsl:template match="para[@role='note']"><note><xsl:value-of select="."/></note></xsl:template>
  <xsl:template match="chapter/para[1]" priority="5"><first><xsl:value-of select="."/></first></xsl:template>
  <xsl:template name="unused"/>
</xsl:stylesheet>
