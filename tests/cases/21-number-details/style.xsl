<?xml version="1.0"?>
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="text"/>
  <xsl:variable name="f" select="'(i)'"/>
  <xsl:template match="/">
    <xsl:number value="0 div 0"/>|<xsl:number value="1 div 0"/>|<xsl:number value="-1 div 0"/>|<xsl:number value="-3"/>|<xsl:number value="2.5"/>|<xsl:number value="3.49999"/>
    |<xsl:number value="1e20"/>|<xsl:number value="123456789012345678901234567890"/>
    |<xsl:number value="0" format="a"/>|<xsl:number value="0" format="I"/>|<xsl:number value="5001" format="I"/>|<xsl:number value="4999" format="I"/>
    |<xsl:number value="702" format="A"/>|<xsl:number value="703" format="a"/>|<xsl:number value="18278" format="a"/>
    |<xsl:number value="7" format="x"/>|<xsl:number value="7" format="2"/>|<xsl:number value="7" format="0"/>|<xsl:number value="7" format="0001"/>|<xsl:number value="7" format="&#x661;"/>|<xsl:number value="12" format="&#x660;&#x661;"/>
    |<xsl:number value="7" format="[1]"/>|<xsl:number value="7" format="."/>|<xsl:number value="7" format="{$f}"/>|<xsl:number value="7" format="#"/>
    |<xsl:number value="1234567" grouping-separator="," grouping-size="3"/>|<xsl:number value="1234567" grouping-separator="," grouping-size="x"/>|<xsl:number value="1234567" grouping-separator="{','}" grouping-size="2"/>|<xsl:number value="1234567" grouping-size="3"/>|<xsl:number value="1234567" grouping-separator="&#xA0;-" grouping-size="2" format="001"/>
    |<xsl:number value="5" level="bogus"/>|<xsl:number value="'abc'"/>|<xsl:number/>|<xsl:number count="nothing"/>|<xsl:number level="any" count="nothing"/>
    <xsl:text>&#10;</xsl:text>
    <!-- One kind at a time: libxml2 sorts a union of text, comment or PI
         nodes and elements in its own order (xmlXPathCmpNodesExt). -->
    <xsl:apply-templates select="//p"/>
    <xsl:apply-templates select="//chap/@t"/>
    <xsl:apply-templates select="//processing-instruction()"/>
    <xsl:apply-templates select="//comment()"/>
    <xsl:apply-templates select="//chap/text()"/>
  </xsl:template>
  <xsl:template match="p">
    <xsl:number/>,<xsl:number level="multiple"/>,<xsl:number level="multiple" count="chap|sec|p" format="1.a.i"/>,<xsl:number level="multiple" count="chap|sec|p" format="I-"/>,<xsl:number level="any"/>,<xsl:number level="any" from="chap"/>,<xsl:number level="any" count="*"/>,<xsl:number level="any" count="node()"/>,<xsl:number level="multiple" count="*" from="chap"/>,<xsl:number count="sec" from="sec"/>,<xsl:number level="single" count="chap"/>;<xsl:text>&#10;</xsl:text>
  </xsl:template>
  <xsl:template match="@t|processing-instruction()|comment()|text()">
    <xsl:value-of select="name()"/>:<xsl:number/>,<xsl:number level="any"/>,<xsl:number level="multiple" count="chap|@t"/>;<xsl:text>&#10;</xsl:text>
  </xsl:template>
</xsl:stylesheet>
