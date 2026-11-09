<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="xml" indent="yes"/>
  <xsl:key name="by-cust" match="o" use="@cust"/>
  <xsl:template match="/orders">
    <groups>
      <xsl:for-each select="o[generate-id() = generate-id(key('by-cust', @cust)[1])]">
        <xsl:sort select="@cust"/>
        <g cust="{@cust}" n="{count(key('by-cust', @cust))}" total="{sum(key('by-cust', @cust)/@amt)}"/>
      </xsl:for-each>
      <same><xsl:value-of select="generate-id(o[1]) = generate-id(o[1])"/></same>
      <diff><xsl:value-of select="generate-id(o[1]) = generate-id(o[2])"/></diff>
    </groups>
  </xsl:template>
</xsl:stylesheet>
