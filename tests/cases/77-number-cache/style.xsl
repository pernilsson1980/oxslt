<!-- xsl:number counts are cached per instruction and node only when its
     patterns depend on nothing but the documents. A pattern with a
     variable (libxslt allows them) must be counted again each time, and
     generate-id() in a predicate must still number nodes as it's asked. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:output indent="yes"/>
<xsl:template match="/">
  <out>
    <xsl:for-each select="//fig">
      <pure n="{@n}"><xsl:call-template name="pure"/>/<xsl:call-template name="pure"/></pure>
    </xsl:for-each>
    <xsl:for-each select="//fig">
      <xsl:call-template name="with-var"><xsl:with-param name="kind" select="'a'"/></xsl:call-template>
      <xsl:call-template name="with-var"><xsl:with-param name="kind" select="'b'"/></xsl:call-template>
    </xsl:for-each>
    <xsl:for-each select="//fig">
      <ids><xsl:number level="any" count="fig[generate-id() != '']"/>:<xsl:value-of select="generate-id()"/></ids>
    </xsl:for-each>
  </out>
</xsl:template>
<xsl:template name="pure"><xsl:number level="any" count="fig[@n != 'skip']" from="chapter"/></xsl:template>
<xsl:template name="with-var">
  <xsl:param name="kind"/>
  <var kind="{$kind}"><xsl:number level="any" count="fig[@kind = $kind]"/></var>
</xsl:template>
</xsl:stylesheet>
